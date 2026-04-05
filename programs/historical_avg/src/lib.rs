pub fn compute_historical_avg_result(balances: &[u64]) -> u64 {
    if balances.is_empty() {
        return 0;
    }

    let sum = balances
        .iter()
        .fold(0u64, |accumulator, &value| accumulator.saturating_add(value));
    sum / balances.len() as u64
}

#[cfg(test)]
mod tests {
    use super::compute_historical_avg_result;

    #[test]
    fn empty_inputs_yield_zero() {
        assert_eq!(compute_historical_avg_result(&[]), 0);
    }

    #[test]
    fn uniform_inputs_preserve_their_value() {
        assert_eq!(compute_historical_avg_result(&[42, 42, 42]), 42);
    }
}