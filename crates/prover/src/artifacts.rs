use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::sync::Mutex;

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sp1_sdk::{
    blocking::{Elf, Prover, ProverClient},
    HashableKey, ProvingKey, SP1_CIRCUIT_VERSION,
};

use crate::{
    registry::{registered_computations, RegisteredComputation},
    sp1_wrapper::{build_sp1_program, configure_prover_environment},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Groth16VerifierArtifact {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputationVerifierArtifact {
    pub schema_version: u32,
    pub computation_name: String,
    pub elf_path: String,
    pub computation_id: String,
    pub sp1_circuit_version: String,
    pub sp1_vk_hash_bytes: String,
    pub sp1_vk_hash_bn254: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groth16_verifier_artifact: Option<Groth16VerifierArtifact>,
}

#[cfg(test)]
static ARTIFACT_ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn export_registered_artifacts_to_dir(
    output_dir: impl AsRef<Path>,
) -> anyhow::Result<Vec<PathBuf>> {
    let computations = registered_computations()?;
    computations
        .iter()
        .map(|computation| export_verifier_artifact(*computation, output_dir.as_ref()))
        .collect()
}

pub fn export_verifier_artifact(
    computation: RegisteredComputation,
    output_dir: impl AsRef<Path>,
) -> anyhow::Result<PathBuf> {
    let artifact = build_verifier_artifact(computation)?;
    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create artifact dir {}", output_dir.display()))?;

    let output_path = output_dir.join(format!("{}_vkey.json", computation.name));
    let json =
        serde_json::to_vec_pretty(&artifact).context("failed to serialize verifier artifact")?;
    fs::write(&output_path, json)
        .with_context(|| format!("failed to write artifact {}", output_path.display()))?;

    Ok(output_path)
}

fn build_verifier_artifact(
    computation: RegisteredComputation,
) -> anyhow::Result<ComputationVerifierArtifact> {
    configure_prover_environment();

    let elf = build_sp1_program(computation.elf_path)?;
    let prover = ProverClient::from_env();
    let pk = prover
        .setup(Elf::from(elf))
        .with_context(|| format!("failed to set up SP1 proving key for {}", computation.name))?;
    let vk = pk.verifying_key();

    Ok(ComputationVerifierArtifact {
        schema_version: 1,
        computation_name: computation.name.to_string(),
        elf_path: computation.elf_path.to_string(),
        computation_id: encode_hex(&computation.computation_id),
        sp1_circuit_version: SP1_CIRCUIT_VERSION.to_string(),
        sp1_vk_hash_bytes: encode_hex(&vk.hash_bytes()),
        sp1_vk_hash_bn254: vk.bytes32(),
        groth16_verifier_artifact: discover_groth16_verifier_artifact()?,
    })
}

fn discover_groth16_verifier_artifact() -> anyhow::Result<Option<Groth16VerifierArtifact>> {
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
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if !name.ends_with("-groth16-dev") {
            continue;
        }

        let vkey_path = path.join("groth16_vk.bin");
        if vkey_path.is_file() {
            candidates.push(vkey_path);
        }
    }

    candidates.sort();
    if candidates.is_empty() {
        return Ok(None);
    }

    let mut selected = None;
    let mut selected_hash = None;
    for candidate in candidates {
        let bytes = fs::read(&candidate)
            .with_context(|| format!("failed to read {}", candidate.display()))?;
        let digest = encode_hex(&Sha256::digest(bytes));

        match &selected_hash {
            None => {
                selected_hash = Some(digest.clone());
                selected = Some(candidate);
            },
            Some(existing) if existing == &digest => {},
            Some(existing) => {
                return Err(anyhow!(
                    "found multiple distinct groth16 verifier artifacts in {} ({} != {})",
                    circuits_dir.display(),
                    existing,
                    digest
                ));
            },
        }
    }

    Ok(selected.map(|path| Groth16VerifierArtifact {
        path: path.display().to_string(),
        sha256: selected_hash.expect("selected artifact hash should exist"),
    }))
}

fn circuits_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SONAR_SP1_CIRCUITS_DIR") {
        return Some(PathBuf::from(path));
    }

    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".sp1").join("circuits"))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::registry::{fibonacci_computation_id, resolve_computation};

    #[test]
    fn test_discovers_groth16_artifact_from_override_dir() {
        let _guard = ARTIFACT_ENV_LOCK
            .lock()
            .expect("artifact env lock should not be poisoned");
        let tempdir = tempdir().expect("tempdir should create");
        let circuits_dir = tempdir.path().join("circuits");
        let build_dir = circuits_dir.join("abcd1234-groth16-dev");
        fs::create_dir_all(&build_dir).expect("build dir should create");

        let vk_bytes = [1_u8, 2, 3, 4, 5];
        fs::write(build_dir.join("groth16_vk.bin"), vk_bytes).expect("vk bytes should write");

        let previous = std::env::var_os("SONAR_SP1_CIRCUITS_DIR");
        std::env::set_var("SONAR_SP1_CIRCUITS_DIR", &circuits_dir);

        let artifact = discover_groth16_verifier_artifact()
            .expect("discovery should succeed")
            .expect("artifact should exist");

        assert!(artifact.path.ends_with("groth16_vk.bin"));
        assert_eq!(artifact.sha256, encode_hex(&Sha256::digest(vk_bytes)));

        if let Some(value) = previous {
            std::env::set_var("SONAR_SP1_CIRCUITS_DIR", value);
        } else {
            std::env::remove_var("SONAR_SP1_CIRCUITS_DIR");
        }
    }

    #[test]
    fn test_exports_registered_fibonacci_artifact_json() {
        let _guard = ARTIFACT_ENV_LOCK
            .lock()
            .expect("artifact env lock should not be poisoned");
        let tempdir = tempdir().expect("tempdir should create");
        let circuits_dir = tempdir.path().join("circuits");
        let build_dir = circuits_dir.join("abcd1234-groth16-dev");
        fs::create_dir_all(&build_dir).expect("build dir should create");
        fs::write(build_dir.join("groth16_vk.bin"), [9_u8, 8, 7, 6])
            .expect("vk bytes should write");

        let previous = std::env::var_os("SONAR_SP1_CIRCUITS_DIR");
        std::env::set_var("SONAR_SP1_CIRCUITS_DIR", &circuits_dir);

        let computation_id = fibonacci_computation_id().expect("fibonacci id should derive");
        let computation = resolve_computation(&computation_id).expect("computation should resolve");
        let output_path = export_verifier_artifact(computation, tempdir.path())
            .expect("artifact export should succeed");

        let json = fs::read_to_string(&output_path).expect("artifact json should be readable");
        let artifact: ComputationVerifierArtifact =
            serde_json::from_str(&json).expect("artifact json should deserialize");

        assert_eq!(artifact.schema_version, 1);
        assert_eq!(artifact.computation_name, "fibonacci");
        assert_eq!(artifact.computation_id, encode_hex(&computation_id));
        assert_eq!(artifact.sp1_vk_hash_bytes.len(), 64);
        assert!(artifact.sp1_vk_hash_bn254.starts_with("0x"));
        assert_eq!(artifact.sp1_vk_hash_bn254.len(), 66);
        assert_eq!(artifact.sp1_circuit_version, SP1_CIRCUIT_VERSION);
        assert!(artifact.groth16_verifier_artifact.is_some());

        if let Some(value) = previous {
            std::env::set_var("SONAR_SP1_CIRCUITS_DIR", value);
        } else {
            std::env::remove_var("SONAR_SP1_CIRCUITS_DIR");
        }
    }
}
