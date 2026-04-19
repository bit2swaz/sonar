pub mod artifacts;
mod callback_fixtures;
pub mod groth16_wrapper;
pub mod registry;
pub mod service;
pub mod sp1_wrapper;

use sonar_common::types::ComputationId;

use crate::{
    callback_fixtures::maybe_fixture_callback_payload,
    groth16_wrapper::{extract_sp1_groth16_payload_from_proof, wrap_stark_to_groth16},
    registry::{resolve_computation, HISTORICAL_AVG_ELF_PATH},
    sp1_wrapper::{
        build_sp1_program, run_historical_avg_program_groth16_proof, run_sp1_program_groth16_proof,
        Sp1Groth16ProofResult,
    },
};

#[cfg(test)]
use crate::sp1_wrapper::{run_historical_avg_program_groth16_only, run_sp1_program_groth16_only};

pub use artifacts::{
    export_registered_artifacts_to_dir, export_verifier_artifact, ComputationVerifierArtifact,
    Groth16VerifierArtifact,
};

pub fn fibonacci_computation_id() -> anyhow::Result<ComputationId> {
    registry::fibonacci_computation_id()
}

pub fn historical_avg_computation_id() -> anyhow::Result<ComputationId> {
    registry::historical_avg_computation_id()
}

pub fn export_registered_artifacts(
    output_dir: impl AsRef<std::path::Path>,
) -> anyhow::Result<Vec<std::path::PathBuf>> {
    export_registered_artifacts_to_dir(output_dir)
}

fn run_computation(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let computation = resolve_computation(computation_id)?;
    let elf = build_sp1_program(computation.elf_path)?;

    if computation.elf_path == HISTORICAL_AVG_ELF_PATH {
        sp1_wrapper::run_historical_avg_program(&elf, inputs)
    } else {
        sp1_wrapper::run_sp1_program(&elf, inputs)
    }
}

#[cfg(test)]
fn run_callback_computation(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let computation = resolve_computation(computation_id)?;
    let elf = build_sp1_program(computation.elf_path)?;

    if computation.elf_path == HISTORICAL_AVG_ELF_PATH {
        run_historical_avg_program_groth16_only(&elf, inputs)
    } else {
        run_sp1_program_groth16_only(&elf, inputs)
    }
}

fn run_callback_proof(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<Sp1Groth16ProofResult> {
    let computation = resolve_computation(computation_id)?;
    let elf = build_sp1_program(computation.elf_path)?;

    if computation.elf_path == HISTORICAL_AVG_ELF_PATH {
        run_historical_avg_program_groth16_proof(&elf, inputs)
    } else {
        run_sp1_program_groth16_proof(&elf, inputs)
    }
}

pub fn prove(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let (result, stark_proof, public_inputs) = run_computation(computation_id, inputs)?;

    let proof = wrap_stark_to_groth16(&stark_proof, std::slice::from_ref(&public_inputs))?;
    Ok((proof, result, public_inputs))
}

pub fn prove_callback_payload(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<Vec<u8>>)> {
    if let Some(payload) = maybe_fixture_callback_payload(computation_id, inputs)? {
        return Ok(payload);
    }

    let proof_result = run_callback_proof(computation_id, inputs)?;
    let payload = extract_sp1_groth16_payload_from_proof(&proof_result.proof)?;

    Ok((payload.proof, proof_result.result, payload.public_inputs))
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use sha2::{Digest, Sha256};
    use sp1_sdk::{SP1Proof, SP1ProofWithPublicValues, SP1PublicValues};
    use sp1_verifier::Groth16Bn254Proof;

    use super::*;
    use crate::{
        callback_fixtures::HISTORICAL_AVG_CALLBACK_FIXTURE_ENV,
        groth16_wrapper::{extract_sp1_groth16_payload, wrap_stark_to_groth16},
        registry::{FIBONACCI_ELF_PATH, HISTORICAL_AVG_ELF_PATH},
        sp1_wrapper::{
            load_proof_bundle, run_historical_avg_program, run_sp1_program, Sp1ProofBundle,
        },
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

    fn synthetic_callback_bundle() -> (Vec<u8>, [u8; 32]) {
        let raw_proof = (0..256u16).map(|value| value as u8).collect::<Vec<_>>();
        let public_values = SP1PublicValues::from(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let public_values_hash = public_values.hash_bn254().to_string();

        let proof = SP1ProofWithPublicValues::new(
            SP1Proof::Groth16(Groth16Bn254Proof {
                public_inputs: [
                    "1".to_string(),
                    public_values_hash,
                    "2".to_string(),
                    "3".to_string(),
                    "4".to_string(),
                ],
                encoded_proof: String::new(),
                raw_proof: hex::encode(raw_proof),
                groth16_vkey_hash: [9u8; 32],
            }),
            public_values,
            "test".to_string(),
        );

        let bundle = Sp1ProofBundle {
            public_values: proof.public_values.as_slice().to_vec(),
            stark_proof: Vec::new(),
            groth16_proof: bincode::serialize(&proof).expect("proof should serialize"),
        };
        let serialized_bundle = bincode::serialize(&bundle).expect("bundle should serialize");

        let mut expected_public_values_hash = [0u8; 32];
        expected_public_values_hash.copy_from_slice(&Sha256::digest(&bundle.public_values));
        expected_public_values_hash[0] &= 0x1F;

        (serialized_bundle, expected_public_values_hash)
    }

    fn sp1_fixture() -> &'static (Vec<u8>, Vec<u8>, Vec<u8>) {
        let _guard = SP1_ENV_LOCK
            .lock()
            .expect("SP1 env lock should not be poisoned");
        SP1_FIXTURE.get_or_init(|| {
            let elf = build_sp1_program(FIBONACCI_ELF_PATH).expect("fibonacci ELF should load");
            run_sp1_program(&elf, &fibonacci_input(10)).expect("SP1 run should succeed")
        })
    }

    fn prove_fixture() -> &'static (Vec<u8>, Vec<u8>, Vec<u8>) {
        let _guard = SP1_ENV_LOCK
            .lock()
            .expect("SP1 env lock should not be poisoned");
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
    fn test_historical_avg_mock_prover_returns_wrappable_bundle() {
        let _guard = SP1_ENV_LOCK
            .lock()
            .expect("SP1 env lock should not be poisoned");
        let previous = std::env::var("SP1_PROVER").ok();
        std::env::set_var("SP1_PROVER", "mock");

        let balances = vec![200_u64, 280_u64, 150_u64, 480_u64];
        let encoded = bincode::serialize(&balances).expect("serialize balances");
        let elf =
            build_sp1_program(HISTORICAL_AVG_ELF_PATH).expect("historical_avg ELF should load");
        let (result, proof, public_inputs) =
            run_historical_avg_program(&elf, &encoded).expect("mock proving should succeed");

        let expected = sp1_wrapper::compute_historical_avg_result(&balances)
            .to_le_bytes()
            .to_vec();
        assert_eq!(result, expected);
        assert_eq!(public_inputs, expected);
        assert!(
            load_proof_bundle(&proof).is_ok(),
            "historical_avg mock proof should be a serialized SP1 proof bundle"
        );

        if let Some(value) = previous {
            std::env::set_var("SP1_PROVER", value);
        } else {
            std::env::remove_var("SP1_PROVER");
        }
    }

    #[test]
    fn test_prove_historical_avg_mock_returns_non_empty_proof() {
        let _guard = SP1_ENV_LOCK
            .lock()
            .expect("SP1 env lock should not be poisoned");
        let previous = std::env::var("SP1_PROVER").ok();
        let previous_fixture = std::env::var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV).ok();
        std::env::set_var("SP1_PROVER", "mock");
        std::env::remove_var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV);

        let computation_id = historical_avg_computation_id().expect("computation id should derive");
        let balances = vec![200_u64, 280_u64, 150_u64, 480_u64];
        let encoded = bincode::serialize(&balances).expect("serialize balances");

        let error = prove_callback_payload(&computation_id, &encoded)
            .expect_err("mock historical avg callback payload should require a real proof");

        assert!(
            error
                .to_string()
                .contains("mock Groth16 proofs do not contain raw proof bytes"),
            "unexpected error: {error:#}"
        );

        if let Some(value) = previous {
            std::env::set_var("SP1_PROVER", value);
        } else {
            std::env::remove_var("SP1_PROVER");
        }

        if let Some(value) = previous_fixture {
            std::env::set_var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV, value);
        } else {
            std::env::remove_var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV);
        }
    }

    #[test]
    fn test_prove_historical_avg_mock_with_callback_fixture_returns_payload() {
        let _guard = SP1_ENV_LOCK
            .lock()
            .expect("SP1 env lock should not be poisoned");
        let previous = std::env::var("SP1_PROVER").ok();
        let previous_fixture = std::env::var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV).ok();
        std::env::set_var("SP1_PROVER", "mock");
        std::env::set_var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV, "1");

        let computation_id = historical_avg_computation_id().expect("computation id should derive");
        let balances = vec![200_u64, 280_u64, 150_u64, 480_u64];
        let encoded = bincode::serialize(&balances).expect("serialize balances");
        let expected = sp1_wrapper::compute_historical_avg_result(&balances)
            .to_le_bytes()
            .to_vec();

        let (proof, result, public_inputs) = prove_callback_payload(&computation_id, &encoded)
            .expect("fixture payload should succeed");

        assert_eq!(proof.len(), 256);
        assert_eq!(result, expected);
        assert_eq!(public_inputs.len(), 9);
        assert!(public_inputs.iter().all(|input| input.len() == 32));

        if let Some(value) = previous {
            std::env::set_var("SP1_PROVER", value);
        } else {
            std::env::remove_var("SP1_PROVER");
        }

        if let Some(value) = previous_fixture {
            std::env::set_var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV, value);
        } else {
            std::env::remove_var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV);
        }
    }

    #[test]
    fn test_extract_sp1_groth16_payload_matches_cached_shape_fixture() {
        let (serialized_bundle, expected_public_values_hash) = synthetic_callback_bundle();
        let payload = extract_sp1_groth16_payload(&serialized_bundle)
            .expect("payload extraction should succeed");

        assert_eq!(payload.proof.len(), 256);
        assert_eq!(payload.public_inputs.len(), 5);
        assert!(payload.public_inputs.iter().all(|input| input.len() == 32));
        assert_eq!(
            payload.public_inputs[1],
            expected_public_values_hash.to_vec()
        );
    }

    #[test]
    #[ignore = "expensive real SP1 proving"]
    fn test_extract_sp1_groth16_payload_matches_real_sp1_shape_live_smoke() {
        let _guard = SP1_ENV_LOCK
            .lock()
            .expect("SP1 env lock should not be poisoned");
        let previous = std::env::var("SP1_PROVER").ok();
        if previous.is_none() {
            std::env::set_var("SP1_PROVER", "cpu");
        }

        let computation_id = fibonacci_computation_id().expect("computation id should derive");
        let (_result, stark_proof, _public_inputs) =
            run_callback_computation(&computation_id, &fibonacci_input(1))
                .expect("real SP1 run should succeed");
        let bundle = load_proof_bundle(&stark_proof).expect("proof bundle should decode");
        let payload =
            extract_sp1_groth16_payload(&stark_proof).expect("payload extraction should succeed");

        let mut expected_public_values_hash = [0u8; 32];
        expected_public_values_hash.copy_from_slice(&Sha256::digest(&bundle.public_values));
        expected_public_values_hash[0] &= 0x1F;

        assert_eq!(payload.proof.len(), 256);
        assert_eq!(payload.public_inputs.len(), 5);
        assert!(payload.public_inputs.iter().all(|input| input.len() == 32));
        assert_eq!(
            payload.public_inputs[1],
            expected_public_values_hash.to_vec()
        );

        if let Some(value) = previous {
            std::env::set_var("SP1_PROVER", value);
        } else {
            std::env::remove_var("SP1_PROVER");
        }
    }
}
