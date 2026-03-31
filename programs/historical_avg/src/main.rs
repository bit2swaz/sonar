//! SP1 guest program — Historical Balance Average.
//!
//! Reads a `Vec<u64>` of lamport balances from stdin (supplied by the
//! Sonar coordinator, which fetched them from the indexer), computes
//! the integer average, and commits the result.
//!
//! Public outputs committed to the proof:
//!   • `u64` — average lamports (0 when the input is empty)

#![no_main]
sp1_zkvm::entrypoint!(main);

pub fn main() {
    let balances: Vec<u64> = sp1_zkvm::io::read();

    let avg: u64 = if balances.is_empty() {
        0
    } else {
        let sum: u64 = balances.iter().fold(0u64, |acc, &x| acc.saturating_add(x));
        sum / balances.len() as u64
    };

    sp1_zkvm::io::commit(&avg);
}
