use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};

use proptest::collection::vec;
use proptest::prelude::*;
use sonar_prover::registry::HISTORICAL_AVG_ELF_PATH;
use sonar_prover::sp1_wrapper::{
    build_sp1_program, compute_historical_avg_result, run_historical_avg_program,
};

static SP1_ENV_LOCK: Mutex<()> = Mutex::new(());
static SP1_ENV_INIT: OnceLock<()> = OnceLock::new();
static HISTORICAL_AVG_ELF: OnceLock<Vec<u8>> = OnceLock::new();

fn historical_avg_elf() -> &'static [u8] {
    HISTORICAL_AVG_ELF.get_or_init(|| {
        build_sp1_program(HISTORICAL_AVG_ELF_PATH).expect("historical_avg ELF should load")
    })
}

fn with_mock_prover<T>(f: impl FnOnce() -> T) -> T {
    let _guard = SP1_ENV_LOCK
        .lock()
        .expect("SP1 env lock should not be poisoned");

    SP1_ENV_INIT.get_or_init(|| {
        std::env::set_var("SP1_PROVER", "mock");
    });
    std::env::set_var("SP1_PROVER", "mock");

    f()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn historical_avg_mock_runner_matches_pure_helper(balances in vec(any::<u64>(), 0..256)) {
        let encoded = bincode::serialize(&balances).expect("historical_avg balances should serialize");

        let outcome = with_mock_prover(|| run_historical_avg_program(historical_avg_elf(), &encoded));
        let (result, proof, public_inputs) = outcome.expect("mock historical_avg runner should succeed for serialized Vec<u64>");

        let expected = compute_historical_avg_result(&balances).to_le_bytes().to_vec();
        prop_assert_eq!(result, expected.clone());
        prop_assert_eq!(public_inputs, expected);
        prop_assert!(!proof.is_empty());
    }

    #[test]
    fn malformed_historical_avg_inputs_never_panic(bytes in vec(any::<u8>(), 0..512)) {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            with_mock_prover(|| run_historical_avg_program(historical_avg_elf(), &bytes))
        }));

        prop_assert!(outcome.is_ok(), "malformed bytes must not panic");

        let result = outcome.expect("catch_unwind should preserve the Result");
        if bincode::deserialize::<Vec<u64>>(&bytes).is_ok() {
            prop_assert!(result.is_ok(), "valid Vec<u64> encodings should succeed in mock mode");
        } else {
            prop_assert!(result.is_err(), "invalid encodings should return an error rather than panic");
        }
    }

    #[test]
    fn saturating_average_stays_within_maximum_balance(balances in vec(any::<u64>(), 1..256)) {
        let average = compute_historical_avg_result(&balances);
        let maximum = balances.iter().copied().max().unwrap_or_default();

        prop_assert!(average <= maximum);
    }
}
