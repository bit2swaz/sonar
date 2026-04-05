use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anchor_lang::{InstructionData, ToAccountMetas};
use anyhow::{anyhow, bail, Context, Result};
use ark_bn254::{Fq, Fq2, G1Affine, G2Affine};
use ark_ff::{BigInteger, PrimeField};
use clap::{Args, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signer},
    transaction::Transaction,
};

const DEFAULT_RPC_URL: &str = "http://127.0.0.1:8899";
const VERIFIER_SEED: &[u8] = b"verifier";
const SPINNER_TICK_MS: u64 = 80;

#[derive(Debug, Parser)]
#[command(
    name = "sonar-cli",
    version,
    about = "Developer CLI for Sonar verifier registration"
)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Register a verifier for a compiled SP1 guest ELF.
    Register(RegisterArgs),
}

#[derive(Debug, Clone, Args)]
pub struct RegisterArgs {
    /// Path to the compiled SP1 guest ELF.
    #[arg(long, value_name = "PATH")]
    pub elf_path: PathBuf,

    /// Path to the authority keypair that will submit register_verifier.
    #[arg(long, value_name = "PATH")]
    pub keypair: PathBuf,

    /// Optional explicit verifier artifact JSON produced by `sonar-export-artifacts`.
    #[arg(long, value_name = "PATH")]
    pub vkey_json: Option<PathBuf>,

    /// Solana RPC URL used to send the register_verifier transaction.
    #[arg(long, env = "SOLANA_RPC_URL", default_value = DEFAULT_RPC_URL, value_name = "URL")]
    pub rpc_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationSummary {
    pub computation_id: [u8; 32],
    pub verifier_registry: Pubkey,
    pub signature: solana_sdk::signature::Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Groth16VerifierArtifact {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ComputationVerifierArtifact {
    computation_id: String,
    #[serde(default)]
    groth16_verifier_artifact: Option<Groth16VerifierArtifact>,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Register(args) => {
            let summary = register(args)?;
            println!(
                "registered verifier {} for computation {}",
                summary.verifier_registry,
                hex_encode(&summary.computation_id)
            );
            println!("signature: {}", summary.signature);
            Ok(())
        },
    }
}

pub fn register(args: RegisterArgs) -> Result<RegistrationSummary> {
    let progress = spinner("Reading ELF and computing computation_id...");

    let elf_bytes = fs::read(&args.elf_path)
        .with_context(|| format!("failed to read ELF {}", args.elf_path.display()))?;
    let computation_id = computation_id_from_elf(&elf_bytes);

    progress.set_message("Resolving Groth16 verifier artifact...");
    let vk_bytes = resolve_groth16_vk_bytes(&args, &computation_id)?;
    let params = register_verifier_params(computation_id, &vk_bytes)?;

    progress.set_message("Loading authority and preparing transaction...");
    let authority = read_keypair_file(&args.keypair)
        .map_err(|error| anyhow!(error.to_string()))
        .with_context(|| format!("failed to read keypair {}", args.keypair.display()))?;

    let rpc = RpcClient::new_with_commitment(args.rpc_url.clone(), CommitmentConfig::confirmed());
    let verifier_registry = verifier_registry_pda(&computation_id);
    let instruction = build_register_verifier_instruction(&authority, verifier_registry, params);

    progress.set_message("Submitting register_verifier transaction...");
    let signature = send_transaction(&rpc, &authority, &[instruction])?;
    progress.finish_with_message("Verifier registered successfully");

    Ok(RegistrationSummary {
        computation_id,
        verifier_registry,
        signature,
    })
}

fn spinner(message: &str) -> ProgressBar {
    let progress = ProgressBar::new_spinner();
    progress.enable_steady_tick(Duration::from_millis(SPINNER_TICK_MS));
    progress.set_style(
        ProgressStyle::with_template("{spinner:.green} {msg}").expect("valid spinner template"),
    );
    progress.set_message(message.to_string());
    progress
}

fn computation_id_from_elf(elf_bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(elf_bytes).into()
}

fn resolve_groth16_vk_bytes(args: &RegisterArgs, computation_id: &[u8; 32]) -> Result<Vec<u8>> {
    if let Some(path) = args
        .vkey_json
        .clone()
        .or_else(|| discover_vkey_json(&args.elf_path))
    {
        return load_vk_bytes_from_artifact_json(&path, computation_id);
    }

    if let Some(path) = discover_groth16_vk_path()? {
        return fs::read(&path).with_context(|| {
            format!(
                "failed to read Groth16 verifier artifact {}",
                path.display()
            )
        });
    }

    Ok(sp1_verifier::GROTH16_VK_BYTES.to_vec())
}

fn discover_vkey_json(elf_path: &Path) -> Option<PathBuf> {
    let stem = elf_path.file_stem()?.to_str()?;
    let file_name = format!("{stem}_vkey.json");

    let direct_sibling = elf_path.with_file_name(&file_name);
    if direct_sibling.is_file() {
        return Some(direct_sibling);
    }

    for ancestor in elf_path.ancestors() {
        let candidate = ancestor.join("artifacts").join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if ancestor.join("Cargo.toml").is_file() {
            break;
        }
    }

    None
}

fn load_vk_bytes_from_artifact_json(
    artifact_json_path: &Path,
    expected_computation_id: &[u8; 32],
) -> Result<Vec<u8>> {
    let json = fs::read_to_string(artifact_json_path)
        .with_context(|| format!("failed to read artifact {}", artifact_json_path.display()))?;
    let artifact: ComputationVerifierArtifact = serde_json::from_str(&json)
        .with_context(|| format!("failed to parse artifact {}", artifact_json_path.display()))?;

    let artifact_computation_id = decode_hex_32(&artifact.computation_id)
        .context("failed to decode artifact computation_id")?;
    if artifact_computation_id != *expected_computation_id {
        bail!(
            "artifact computation_id {} does not match ELF SHA256 {}",
            artifact.computation_id,
            hex_encode(expected_computation_id)
        );
    }

    let groth16_artifact = artifact.groth16_verifier_artifact.ok_or_else(|| {
        anyhow!(
            "artifact {} does not include a groth16_verifier_artifact path",
            artifact_json_path.display()
        )
    })?;

    let candidate_path = PathBuf::from(&groth16_artifact.path);
    let resolved_path = if candidate_path.is_absolute() {
        candidate_path
    } else {
        artifact_json_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(candidate_path)
    };

    let vk_bytes = fs::read(&resolved_path).with_context(|| {
        format!(
            "failed to read Groth16 verifier artifact {}",
            resolved_path.display()
        )
    })?;
    let digest = hex_encode(&Sha256::digest(&vk_bytes));
    if digest != groth16_artifact.sha256 {
        bail!(
            "Groth16 verifier artifact hash mismatch for {} (expected {}, got {})",
            resolved_path.display(),
            groth16_artifact.sha256,
            digest
        );
    }

    Ok(vk_bytes)
}

fn discover_groth16_vk_path() -> Result<Option<PathBuf>> {
    let circuits_dir = match circuits_dir() {
        Some(path) => path,
        None => return Ok(None),
    };

    if !circuits_dir.exists() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    for entry in fs::read_dir(&circuits_dir)
        .with_context(|| format!("failed to read SP1 circuits dir {}", circuits_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        if !name.ends_with("-groth16-dev") {
            continue;
        }

        let vk_path = path.join("groth16_vk.bin");
        if vk_path.is_file() {
            candidates.push(vk_path);
        }
    }

    candidates.sort();
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut selected = None;
    let mut selected_hash = None;
    for candidate in candidates {
        let digest = hex_encode(&Sha256::digest(
            fs::read(&candidate)
                .with_context(|| format!("failed to read {}", candidate.display()))?,
        ));

        match &selected_hash {
            None => {
                selected_hash = Some(digest);
                selected = Some(candidate);
            },
            Some(existing) if existing == &digest => {},
            Some(existing) => {
                bail!(
                    "found multiple distinct groth16_vk.bin artifacts in {} ({} != {})",
                    circuits_dir.display(),
                    existing,
                    digest
                );
            },
        }
    }

    Ok(selected)
}

fn circuits_dir() -> Option<PathBuf> {
    if let Some(path) = env::var_os("SONAR_SP1_CIRCUITS_DIR") {
        return Some(PathBuf::from(path));
    }

    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".sp1").join("circuits"))
}

fn register_verifier_params(
    computation_id: [u8; 32],
    vk_bytes: &[u8],
) -> Result<sonar_program::RegisterVerifierParams> {
    let ark_vk = sp1_verifier::load_ark_groth16_verifying_key_from_bytes(vk_bytes)
        .map_err(|error| anyhow!(error))
        .context("failed to decode groth16_vk.bin")?;

    let vk_ic = ark_vk
        .gamma_abc_g1
        .iter()
        .map(g1_affine_to_uncompressed_bytes)
        .collect::<Result<Vec<_>>>()?;

    Ok(sonar_program::RegisterVerifierParams {
        computation_id,
        vk_alpha_g1: g1_affine_to_uncompressed_bytes(&ark_vk.alpha_g1)?,
        vk_beta_g2: g2_affine_to_uncompressed_bytes(&ark_vk.beta_g2)?,
        vk_gamme_g2: g2_affine_to_uncompressed_bytes(&ark_vk.gamma_g2)?,
        vk_delta_g2: g2_affine_to_uncompressed_bytes(&ark_vk.delta_g2)?,
        vk_ic,
    })
}

fn g1_affine_to_uncompressed_bytes(point: &G1Affine) -> Result<[u8; 64]> {
    let mut output = [0u8; 64];
    write_fq_be(&point.x, &mut output[..32]);
    write_fq_be(&point.y, &mut output[32..]);
    Ok(output)
}

fn g2_affine_to_uncompressed_bytes(point: &G2Affine) -> Result<[u8; 128]> {
    let mut output = [0u8; 128];
    write_fq2_be(&point.x, &mut output[..64]);
    write_fq2_be(&point.y, &mut output[64..]);
    Ok(output)
}

fn write_fq2_be(value: &Fq2, output: &mut [u8]) {
    write_fq_be(&value.c1, &mut output[..32]);
    write_fq_be(&value.c0, &mut output[32..64]);
}

fn write_fq_be(value: &Fq, output: &mut [u8]) {
    let bytes = value.into_bigint().to_bytes_be();
    let start = output.len().saturating_sub(bytes.len());
    output.fill(0);
    output[start..start + bytes.len()].copy_from_slice(&bytes);
}

fn verifier_registry_pda(computation_id: &[u8; 32]) -> Pubkey {
    Pubkey::find_program_address(
        &[VERIFIER_SEED, computation_id.as_ref()],
        &sonar_program::ID,
    )
    .0
}

fn build_register_verifier_instruction(
    authority: &Keypair,
    verifier_registry: Pubkey,
    params: sonar_program::RegisterVerifierParams,
) -> Instruction {
    let accounts = sonar_program::accounts::RegisterVerifier {
        authority: authority.pubkey(),
        verifier_registry,
        system_program: anchor_lang::system_program::ID,
    };

    Instruction {
        program_id: sonar_program::ID,
        accounts: accounts.to_account_metas(None),
        data: sonar_program::instruction::RegisterVerifier { params }.data(),
    }
}

fn send_transaction(
    rpc: &RpcClient,
    authority: &Keypair,
    instructions: &[Instruction],
) -> Result<solana_sdk::signature::Signature> {
    let blockhash = rpc.get_latest_blockhash().context("get latest blockhash")?;
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&authority.pubkey()),
        &[authority],
        blockhash,
    );

    // Solana's wire protocol imposes a hard 1232-byte limit per packet.
    // Detect an oversized transaction before attempting submission so the
    // caller receives an actionable error instead of a confusing RPC failure.
    // This commonly occurs when `register_verifier` embeds a large Groth16
    // verifying key (especially `vk_ic`) in a single instruction.
    const SOLANA_MAX_TRANSACTION_BYTES: usize = 1232;
    let wire_size = bincode::serialized_size(&transaction)
        .context("failed to estimate transaction wire size")? as usize;
    if wire_size > SOLANA_MAX_TRANSACTION_BYTES {
        bail!(
            "transaction is too large to submit ({wire_size} bytes; max {SOLANA_MAX_TRANSACTION_BYTES}). \
             This commonly happens when register_verifier embeds a large Groth16 verifying key. \
             Consider registering via chunked writes or a more compact key representation."
        );
    }

    rpc.send_and_confirm_transaction(&transaction)
        .context("send and confirm transaction")
}

fn decode_hex_32(input: &str) -> Result<[u8; 32]> {
    let normalized = input.trim().trim_start_matches("0x");
    if normalized.len() != 64 {
        bail!("expected 64 hex chars, got {}", normalized.len());
    }

    let mut output = [0u8; 32];
    for (index, chunk) in normalized.as_bytes().chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(chunk).context("hex input should be utf-8")?;
        output[index] =
            u8::from_str_radix(hex, 16).with_context(|| format!("invalid hex byte {hex}"))?;
    }
    Ok(output)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn computation_id_is_sha256_of_elf() {
        let bytes = b"fake-elf";
        let expected: [u8; 32] = Sha256::digest(bytes).into();
        assert_eq!(computation_id_from_elf(bytes), expected);
    }

    #[test]
    fn discovers_vkey_json_in_workspace_artifacts_dir() {
        let tempdir = tempdir().expect("tempdir should create");
        let workspace_root = tempdir.path();
        let program_dir = workspace_root.join("programs");
        let artifacts_dir = workspace_root.join("artifacts");
        fs::create_dir_all(&program_dir).expect("program dir should create");
        fs::create_dir_all(&artifacts_dir).expect("artifact dir should create");

        let elf_path = program_dir.join("historical_avg");
        fs::write(&elf_path, b"elf").expect("elf should write");

        let artifact_path = artifacts_dir.join("historical_avg_vkey.json");
        fs::write(&artifact_path, b"{}").expect("artifact should write");

        assert_eq!(discover_vkey_json(&elf_path), Some(artifact_path));
    }

    #[test]
    fn loads_vk_bytes_from_artifact_and_checks_hash() {
        let tempdir = tempdir().expect("tempdir should create");
        let vk_path = tempdir.path().join("groth16_vk.bin");
        let vk_bytes = b"vk-bytes";
        fs::write(&vk_path, vk_bytes).expect("vk bytes should write");

        let computation_id = [7u8; 32];
        let artifact_path = tempdir.path().join("historical_avg_vkey.json");
        let json = serde_json::json!({
            "computation_id": hex_encode(&computation_id),
            "groth16_verifier_artifact": {
                "path": vk_path.display().to_string(),
                "sha256": hex_encode(&Sha256::digest(vk_bytes)),
            }
        });
        fs::write(&artifact_path, serde_json::to_vec(&json).unwrap())
            .expect("artifact json should write");

        let loaded = load_vk_bytes_from_artifact_json(&artifact_path, &computation_id)
            .expect("artifact should load");
        assert_eq!(loaded, vk_bytes);
    }

    #[test]
    fn converts_sp1_groth16_vk_bytes_into_register_params() {
        let params = register_verifier_params([9u8; 32], &sp1_verifier::GROTH16_VK_BYTES)
            .expect("sp1 verifier bytes should parse");

        assert_eq!(params.computation_id, [9u8; 32]);
        assert_eq!(params.vk_alpha_g1.len(), 64);
        assert_eq!(params.vk_beta_g2.len(), 128);
        assert_eq!(params.vk_gamme_g2.len(), 128);
        assert_eq!(params.vk_delta_g2.len(), 128);
        assert!(!params.vk_ic.is_empty());
        assert!(params.vk_ic.iter().all(|entry| entry.len() == 64));
    }
}
