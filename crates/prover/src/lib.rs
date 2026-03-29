pub mod groth16_wrapper;
pub mod sp1_wrapper;

use anyhow::{bail, Context};
use sha2::{Digest, Sha256};
use sonar_common::types::ComputationId;

use crate::{groth16_wrapper::wrap_stark_to_groth16, sp1_wrapper::build_sp1_program};

pub const FIBONACCI_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../programs/fibonacci/elf/fibonacci-program"
);

pub fn fibonacci_computation_id() -> anyhow::Result<ComputationId> {
    computation_id_for_elf(FIBONACCI_ELF_PATH)
}

pub fn prove(computation_id: &[u8; 32], inputs: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let elf_path = resolve_elf_path(computation_id)?;
    let elf = build_sp1_program(elf_path)?;
    let (result, stark_proof) = sp1_wrapper::run_sp1_program(&elf, inputs)?;
    let proof = wrap_stark_to_groth16(&stark_proof, std::slice::from_ref(&result))?;
    Ok((proof, result))
}

fn resolve_elf_path(computation_id: &[u8; 32]) -> anyhow::Result<&'static str> {
    if *computation_id == fibonacci_computation_id()? {
        return Ok(FIBONACCI_ELF_PATH);
    }

    bail!("unknown computation id")
}

fn computation_id_for_elf(elf_path: &str) -> anyhow::Result<ComputationId> {
    let elf = build_sp1_program(elf_path)
        .with_context(|| format!("failed to compute computation id for {elf_path}"))?;
    let digest = Sha256::digest(elf);
    Ok(digest.into())
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::{groth16_wrapper::wrap_stark_to_groth16, sp1_wrapper::run_sp1_program};

    static SP1_FIXTURE: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();
    static PROVE_FIXTURE: OnceLock<(Vec<u8>, Vec<u8>)> = OnceLock::new();

    fn fibonacci_input(n: u32) -> [u8; 4] {
        n.to_le_bytes()
    }

    fn decode_result(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes.try_into().expect("result should be a 4-byte integer"))
    }

    fn sp1_fixture() -> &'static (Vec<u8>, Vec<u8>) {
        SP1_FIXTURE.get_or_init(|| {
            let elf = build_sp1_program(FIBONACCI_ELF_PATH).expect("fibonacci ELF should load");
            run_sp1_program(&elf, &fibonacci_input(10)).expect("SP1 run should succeed")
        })
    }

    fn prove_fixture() -> &'static (Vec<u8>, Vec<u8>) {
        PROVE_FIXTURE.get_or_init(|| {
            let computation_id = fibonacci_computation_id().expect("computation id should derive");
            prove(&computation_id, &fibonacci_input(10)).expect("prove should succeed")
        })
    }

    #[test]
    fn test_sp1_fibonacci() {
        let (result, stark_proof) = sp1_fixture();

        assert_eq!(decode_result(result), 55);
        assert!(
            !stark_proof.is_empty(),
            "SP1 proof bundle should not be empty"
        );
    }

    #[test]
    fn test_groth16_wrapping() {
        let (result, stark_proof) = sp1_fixture();

        let groth16 = wrap_stark_to_groth16(stark_proof, std::slice::from_ref(result))
            .expect("Groth16 wrapping should work");
        assert!(
            !groth16.is_empty(),
            "wrapped Groth16 proof should not be empty"
        );
    }

    #[test]
    fn test_prove_end_to_end() {
        let (proof, result) = prove_fixture();

        assert_eq!(decode_result(result), 55);
        assert!(!proof.is_empty(), "end-to-end proof should not be empty");
    }
}
