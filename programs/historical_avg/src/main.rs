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

use historical_avg_program::compute_historical_avg_result;

pub fn main() {
    let balances: Vec<u64> = sp1_zkvm::io::read();
    let avg = compute_historical_avg_result(&balances);

    sp1_zkvm::io::commit(&avg);
}
