use historical_avg_program::compute_historical_avg_result;
use proptest::collection::vec;
use proptest::prelude::*;

prop_compose! {
    fn uniform_balances_strategy()
        (len in 1usize..128)
        (value in 0u64..=u64::MAX / len as u64, len in Just(len)) -> (u64, usize) {
            (value, len)
        }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn computes_without_panicking_for_arbitrary_balances(balances in vec(any::<u64>(), 0..256)) {
        let expected = if balances.is_empty() {
            0
        } else {
            balances
                .iter()
                .fold(0u64, |accumulator, &value| accumulator.saturating_add(value))
                / balances.len() as u64
        };

        prop_assert_eq!(compute_historical_avg_result(&balances), expected);
    }

    #[test]
    fn average_stays_within_input_bounds(balances in vec(any::<u64>(), 1..256)) {
        let average = compute_historical_avg_result(&balances);
        let maximum = balances.iter().copied().max().unwrap_or_default();

        prop_assert!(average <= maximum);
    }

    #[test]
    fn uniform_balances_round_trip((value, len) in uniform_balances_strategy()) {
        let balances = vec![value; len];
        prop_assert_eq!(compute_historical_avg_result(&balances), value);
    }
}