use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sp1_sdk::{
    blocking::{Elf, ProveRequest, Prover, ProverClient, SP1Stdin},
    ProvingKey, SP1ProofWithPublicValues,
};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sp1ProofBundle {
    pub public_values: Vec<u8>,
    pub stark_proof: Vec<u8>,
    pub groth16_proof: Vec<u8>,
}

pub fn build_sp1_program(elf_path: &str) -> anyhow::Result<Vec<u8>> {
    fs::read(elf_path).with_context(|| format!("failed to load SP1 ELF from {elf_path}"))
}

pub fn run_sp1_program(elf: &[u8], inputs: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let n = decode_fibonacci_input(inputs)?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    let prover = ProverClient::from_env();
    let elf = Elf::from(elf.to_vec());
    let pk = prover
        .setup(elf.clone())
        .context("failed to set up SP1 proving key")?;
    let (public_values, _report) = prover
        .execute(elf.clone(), fibonacci_stdin(n))
        .run()
        .context("failed to execute SP1 program")?;

    let compressed_proof = prover
        .prove(&pk, fibonacci_stdin(n))
        .compressed()
        .run()
        .context("failed to generate SP1 compressed proof")?;
    if !mock_prover {
        prover
            .verify(&compressed_proof, pk.verifying_key(), None)
            .context("failed to verify SP1 compressed proof")?;
    }

    let groth16_proof = prover
        .prove(&pk, fibonacci_stdin(n))
        .groth16()
        .run()
        .context("failed to generate SP1 Groth16 proof")?;
    if !mock_prover {
        prover
            .verify(&groth16_proof, pk.verifying_key(), None)
            .context("failed to verify SP1 Groth16 proof")?;
    }

    let bundle = Sp1ProofBundle {
        public_values: public_values.as_slice().to_vec(),
        stark_proof: serialize_proof(&compressed_proof)?,
        groth16_proof: serialize_proof(&groth16_proof)?,
    };

    let result = fibonacci_result(n).to_le_bytes().to_vec();

    Ok((result.clone(), bincode::serialize(&bundle)?, result))
}

/// Run the historical-average SP1 program.
///
/// `inputs` must be a bincode-encoded `Vec<u64>` (the lamport balances fetched
/// from the indexer by the coordinator).  The returned result is the 8-byte
/// little-endian average.
pub fn run_historical_avg_program(
    elf: &[u8],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let balances: Vec<u64> = bincode::deserialize(inputs)
        .context("failed to deserialize historical_avg inputs as Vec<u64>")?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    let prover = ProverClient::from_env();
    let elf_obj = Elf::from(elf.to_vec());
    let pk = prover
        .setup(elf_obj.clone())
        .context("failed to set up SP1 proving key for historical_avg")?;

    let make_stdin = || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&balances);
        stdin
    };

    let (public_values, _report) = prover
        .execute(elf_obj.clone(), make_stdin())
        .run()
        .context("failed to execute historical_avg SP1 program")?;

    let compressed_proof = prover
        .prove(&pk, make_stdin())
        .compressed()
        .run()
        .context("failed to generate historical_avg SP1 compressed proof")?;
    if !mock_prover {
        prover
            .verify(&compressed_proof, pk.verifying_key(), None)
            .context("failed to verify historical_avg SP1 compressed proof")?;
    }

    let groth16_proof = prover
        .prove(&pk, make_stdin())
        .groth16()
        .run()
        .context("failed to generate historical_avg SP1 Groth16 proof")?;
    if !mock_prover {
        prover
            .verify(&groth16_proof, pk.verifying_key(), None)
            .context("failed to verify historical_avg SP1 Groth16 proof")?;
    }

    let bundle = Sp1ProofBundle {
        public_values: public_values.as_slice().to_vec(),
        stark_proof: serialize_proof(&compressed_proof)?,
        groth16_proof: serialize_proof(&groth16_proof)?,
    };

    let result = compute_historical_avg_result(&balances);

    let result_bytes = result.to_le_bytes().to_vec();

    Ok((
        result_bytes.clone(),
        bincode::serialize(&bundle)?,
        result_bytes,
    ))
}

/// Compute the integer average of `balances` — mirrors the SP1 guest logic.
pub fn compute_historical_avg_result(balances: &[u64]) -> u64 {
    if balances.is_empty() {
        return 0;
    }

    let sum: u64 = balances.iter().fold(0u64, |accumulator, &value| {
        accumulator.saturating_add(value)
    });
    sum / balances.len() as u64
}

pub fn load_proof_bundle(stark_proof: &[u8]) -> anyhow::Result<Sp1ProofBundle> {
    bincode::deserialize(stark_proof).context("failed to decode SP1 proof bundle")
}

pub fn deserialize_proof(bytes: &[u8]) -> anyhow::Result<SP1ProofWithPublicValues> {
    let path = temp_artifact_path("proof-with-public-values.bin");
    fs::write(&path, bytes)
        .with_context(|| format!("failed to stage proof at {}", path.display()))?;

    let proof = SP1ProofWithPublicValues::load(&path)
        .with_context(|| format!("failed to load proof from {}", path.display()));
    let _ = fs::remove_file(&path);
    proof
}

pub(crate) fn configure_prover_environment() {
    if std::env::var("SP1_PROVER").is_ok() {
        return;
    }

    let prover = if cfg!(any(test, feature = "mock")) {
        "mock"
    } else {
        "cpu"
    };
    std::env::set_var("SP1_PROVER", prover);
}

fn using_mock_prover() -> bool {
    std::env::var("SP1_PROVER")
        .map(|value| value.eq_ignore_ascii_case("mock"))
        .unwrap_or(false)
}

fn fibonacci_stdin(n: u32) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    stdin.write(&n);
    stdin
}

fn fibonacci_result(n: u32) -> u32 {
    let mut a = 0u32;
    let mut b = 1u32;
    for _ in 0..n {
        let mut c = a + b;
        c %= 7919;
        a = b;
        b = c;
    }
    a
}

fn decode_fibonacci_input(inputs: &[u8]) -> anyhow::Result<u32> {
    let bytes: [u8; 4] = inputs
        .try_into()
        .map_err(|_| anyhow!("expected 4-byte little-endian fibonacci input"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn serialize_proof(proof: &SP1ProofWithPublicValues) -> anyhow::Result<Vec<u8>> {
    let path = temp_artifact_path("proof-with-public-values.bin");
    proof
        .save(&path)
        .with_context(|| format!("failed to save proof to {}", path.display()))?;
    let bytes = fs::read(&path)
        .with_context(|| format!("failed to read proof bytes from {}", path.display()))?;
    let _ = fs::remove_file(&path);
    Ok(bytes)
}

fn temp_artifact_path(file_name: &str) -> PathBuf {
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    std::env::temp_dir().join(format!("sonar-sp1-{nanos}-{sequence}-{file_name}"))
}
