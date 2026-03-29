use anyhow::{bail, Context};
use sha2::{Digest, Sha256};
use sonar_common::types::ComputationId;

use crate::sp1_wrapper::build_sp1_program;

pub const FIBONACCI_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../programs/fibonacci/elf/fibonacci-program"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredComputation {
    pub name: &'static str,
    pub elf_path: &'static str,
    pub computation_id: ComputationId,
}

pub fn fibonacci_computation_id() -> anyhow::Result<ComputationId> {
    computation_id_for_elf(FIBONACCI_ELF_PATH)
}

pub fn resolve_computation(computation_id: &[u8; 32]) -> anyhow::Result<RegisteredComputation> {
    let fibonacci_id = fibonacci_computation_id()?;
    if *computation_id == fibonacci_id {
        return Ok(RegisteredComputation {
            name: "fibonacci",
            elf_path: FIBONACCI_ELF_PATH,
            computation_id: fibonacci_id,
        });
    }

    bail!("unknown computation id")
}

pub fn computation_id_for_elf(elf_path: &str) -> anyhow::Result<ComputationId> {
    let elf = build_sp1_program(elf_path)
        .with_context(|| format!("failed to compute computation id for {elf_path}"))?;
    let digest = Sha256::digest(elf);
    Ok(digest.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_registered_fibonacci_program() {
        let computation_id = fibonacci_computation_id().expect("fibonacci ID should resolve");
        let entry =
            resolve_computation(&computation_id).expect("registered computation should resolve");

        assert_eq!(entry.name, "fibonacci");
        assert_eq!(entry.elf_path, FIBONACCI_ELF_PATH);
        assert_eq!(entry.computation_id, computation_id);
    }

    #[test]
    fn test_unknown_computation_id_is_rejected() {
        let error = resolve_computation(&[7u8; 32]).expect_err("unknown computation should fail");
        assert!(error.to_string().contains("unknown computation id"));
    }
}
