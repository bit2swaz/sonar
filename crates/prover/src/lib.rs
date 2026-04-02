pub mod groth16_wrapper;
pub mod registry;
pub mod service;
pub mod sp1_wrapper;

use sonar_common::types::ComputationId;

use crate::{
    groth16_wrapper::wrap_stark_to_groth16,
    registry::{resolve_computation, HISTORICAL_AVG_ELF_PATH},
    sp1_wrapper::build_sp1_program,
};

pub fn fibonacci_computation_id() -> anyhow::Result<ComputationId> {
    registry::fibonacci_computation_id()
}

pub fn historical_avg_computation_id() -> anyhow::Result<ComputationId> {
    registry::historical_avg_computation_id()
}

pub fn prove(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let computation = resolve_computation(computation_id)?;
    let elf = build_sp1_program(computation.elf_path)?;

    let (result, stark_proof, public_inputs) = if computation.elf_path == HISTORICAL_AVG_ELF_PATH {
        sp1_wrapper::run_historical_avg_program(&elf, inputs)?
    } else {
        sp1_wrapper::run_sp1_program(&elf, inputs)?
    };

    if computation.elf_path == HISTORICAL_AVG_ELF_PATH
        && std::env::var("SP1_PROVER")
            .map(|value| value.eq_ignore_ascii_case("mock"))
            .unwrap_or(false)
    {
        return Ok((stark_proof, result, public_inputs));
    }

    let proof = wrap_stark_to_groth16(&stark_proof, std::slice::from_ref(&public_inputs))?;
    Ok((proof, result, public_inputs))
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;
    use crate::{
        groth16_wrapper::wrap_stark_to_groth16,
        registry::FIBONACCI_ELF_PATH,
        sp1_wrapper::{mock_historical_avg_proof, run_historical_avg_program, run_sp1_program},
    };

    static SP1_FIXTURE: OnceLock<(Vec<u8>, Vec<u8>, Vec<u8>)> = OnceLock::new();
    static PROVE_FIXTURE: OnceLock<(Vec<u8>, Vec<u8>, Vec<u8>)> = OnceLock::new();
    static SP1_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn fibonacci_input(n: u32) -> [u8; 4] {
        n.to_le_bytes()
    }

    fn decode_result(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes.try_into().expect("result should be a 4-byte integer"))
    }

    fn sp1_fixture() -> &'static (Vec<u8>, Vec<u8>, Vec<u8>) {
        let _guard = SP1_ENV_LOCK.lock().expect("SP1 env lock should not be poisoned");
        SP1_FIXTURE.get_or_init(|| {
            let elf = build_sp1_program(FIBONACCI_ELF_PATH).expect("fibonacci ELF should load");
            run_sp1_program(&elf, &fibonacci_input(10)).expect("SP1 run should succeed")
        })
    }

    fn prove_fixture() -> &'static (Vec<u8>, Vec<u8>, Vec<u8>) {
        let _guard = SP1_ENV_LOCK.lock().expect("SP1 env lock should not be poisoned");
        PROVE_FIXTURE.get_or_init(|| {
            let computation_id = fibonacci_computation_id().expect("computation id should derive");
            prove(&computation_id, &fibonacci_input(10)).expect("prove should succeed")
        })
    }

    #[test]
    fn test_sp1_fibonacci() {
        let (result, stark_proof, public_inputs) = sp1_fixture();

        assert_eq!(decode_result(result), 55);
        assert_eq!(public_inputs.as_slice(), result.as_slice());
        assert!(
            !stark_proof.is_empty(),
            "SP1 proof bundle should not be empty"
        );
    }

    #[test]
    fn test_groth16_wrapping() {
        let (_result, stark_proof, public_inputs) = sp1_fixture();

        let groth16 = wrap_stark_to_groth16(stark_proof, std::slice::from_ref(public_inputs))
            .expect("Groth16 wrapping should work");
        assert!(
            !groth16.is_empty(),
            "wrapped Groth16 proof should not be empty"
        );
    }

    #[test]
    fn test_prove_end_to_end() {
        let (proof, result, public_inputs) = prove_fixture();

        assert_eq!(decode_result(result), 55);
        assert_eq!(public_inputs.as_slice(), result.as_slice());
        assert!(!proof.is_empty(), "end-to-end proof should not be empty");
    }

    // ---------- historical_avg helpers ----------

    #[test]
    fn test_compute_historical_avg_result_empty() {
        assert_eq!(sp1_wrapper::compute_historical_avg_result(&[]), 0);
    }

    #[test]
    fn test_compute_historical_avg_result_single() {
        assert_eq!(sp1_wrapper::compute_historical_avg_result(&[42]), 42);
    }

    #[test]
    fn test_compute_historical_avg_result_multiple() {
        // [100, 200, 300] → sum=600, avg=200
        assert_eq!(
            sp1_wrapper::compute_historical_avg_result(&[100, 200, 300]),
            200
        );
    }

    #[test]
    fn test_compute_historical_avg_result_truncates() {
        // [1, 2] → sum=3, avg=1 (integer div)
        assert_eq!(sp1_wrapper::compute_historical_avg_result(&[1, 2]), 1);
    }

    #[test]
    fn test_historical_avg_mock_prover_short_circuits_heavy_proving() {
        let _guard = SP1_ENV_LOCK.lock().expect("SP1 env lock should not be poisoned");
        let previous = std::env::var("SP1_PROVER").ok();
        std::env::set_var("SP1_PROVER", "mock");

        let balances = vec![200_u64, 280_u64, 150_u64, 480_u64];
        let encoded = bincode::serialize(&balances).expect("serialize balances");
        let (result, proof, public_inputs) =
            run_historical_avg_program(&[0_u8; 4], &encoded).expect("mock proving should succeed");

        let expected = sp1_wrapper::compute_historical_avg_result(&balances)
            .to_le_bytes()
            .to_vec();
        assert_eq!(result, expected);
        assert_eq!(public_inputs, expected);
        assert_eq!(proof, mock_historical_avg_proof(&expected));

        if let Some(value) = previous {
            std::env::set_var("SP1_PROVER", value);
        } else {
            std::env::remove_var("SP1_PROVER");
        }
    }

    #[test]
    fn test_prove_historical_avg_mock_skips_groth16_wrap() {
        let _guard = SP1_ENV_LOCK.lock().expect("SP1 env lock should not be poisoned");
        let previous = std::env::var("SP1_PROVER").ok();
        std::env::set_var("SP1_PROVER", "mock");

        let computation_id = historical_avg_computation_id().expect("computation id should derive");
        let balances = vec![200_u64, 280_u64, 150_u64, 480_u64];
        let encoded = bincode::serialize(&balances).expect("serialize balances");

        let (proof, result, public_inputs) =
            prove(&computation_id, &encoded).expect("mock historical avg prove should succeed");

        let expected = sp1_wrapper::compute_historical_avg_result(&balances)
            .to_le_bytes()
            .to_vec();
        assert_eq!(result, expected);
        assert_eq!(public_inputs, expected);
        assert_eq!(proof, mock_historical_avg_proof(&expected));

        if let Some(value) = previous {
            std::env::set_var("SP1_PROVER", value);
        } else {
            std::env::remove_var("SP1_PROVER");
        }
    }
}
