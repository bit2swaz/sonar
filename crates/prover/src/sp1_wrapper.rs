use std::fs;

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sp1_sdk::{
    blocking::{Elf, ProveRequest, Prover, ProverClient, SP1Stdin},
    ProofFromNetwork, ProvingKey, SP1ProofWithPublicValues,
};

const DEFAULT_LOCAL_SP1_SHARD_SIZE: &str = "250000";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sp1ProofBundle {
    pub public_values: Vec<u8>,
    pub stark_proof: Vec<u8>,
    pub groth16_proof: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct Sp1Groth16ProofResult {
    pub result: Vec<u8>,
    pub proof: SP1ProofWithPublicValues,
}

pub fn build_sp1_program(elf_path: &str) -> anyhow::Result<Vec<u8>> {
    fs::read(elf_path).with_context(|| format!("failed to load SP1 ELF from {elf_path}"))
}

pub fn run_sp1_program(elf: &[u8], inputs: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    run_sp1_program_internal(elf, inputs, true, true)
}

#[cfg(test)]
pub(crate) fn run_sp1_program_groth16_only(
    elf: &[u8],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    run_sp1_program_internal(elf, inputs, false, should_verify_callback_proofs_locally())
}

pub(crate) fn run_sp1_program_groth16_proof(
    elf: &[u8],
    inputs: &[u8],
) -> anyhow::Result<Sp1Groth16ProofResult> {
    run_sp1_program_groth16_proof_internal(
        elf,
        inputs,
        should_verify_callback_proofs_locally(),
    )
}

fn run_sp1_program_groth16_proof_internal(
    elf: &[u8],
    inputs: &[u8],
    verify_proofs: bool,
) -> anyhow::Result<Sp1Groth16ProofResult> {
    let n = decode_fibonacci_input(inputs)?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    let prover = ProverClient::from_env();
    let pk = prover
        .setup(Elf::from(elf.to_vec()))
        .context("failed to set up SP1 proving key")?;
    let groth16_proof = prover
        .prove(&pk, fibonacci_stdin(n))
        .groth16()
        .run()
        .context("failed to generate SP1 Groth16 proof")?;
    if !mock_prover && verify_proofs {
        prover
            .verify(&groth16_proof, pk.verifying_key(), None)
            .context("failed to verify SP1 Groth16 proof")?;
    }

    Ok(Sp1Groth16ProofResult {
        result: fibonacci_result(n).to_le_bytes().to_vec(),
        proof: groth16_proof,
    })
}

fn run_sp1_program_internal(
    elf: &[u8],
    inputs: &[u8],
    include_compressed_proof: bool,
    verify_proofs: bool,
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if !include_compressed_proof {
        let proof_result = run_sp1_program_groth16_proof_internal(elf, inputs, verify_proofs)?;
        let result = proof_result.result;
        let bundle = Sp1ProofBundle {
            public_values: proof_result.proof.public_values.as_slice().to_vec(),
            stark_proof: Vec::new(),
            groth16_proof: serialize_proof(&proof_result.proof)?,
        };

        return Ok((result.clone(), bincode::serialize(&bundle)?, result));
    }

    let n = decode_fibonacci_input(inputs)?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    let prover = ProverClient::from_env();
    let elf = Elf::from(elf.to_vec());
    let pk = prover
        .setup(elf.clone())
        .context("failed to set up SP1 proving key")?;

    let public_values = if include_compressed_proof {
        let (public_values, _report) = prover
            .execute(elf.clone(), fibonacci_stdin(n))
            .run()
            .context("failed to execute SP1 program")?;
        public_values.as_slice().to_vec()
    } else {
        Vec::new()
    };

    let compressed_proof = if include_compressed_proof {
        let compressed_proof = prover
            .prove(&pk, fibonacci_stdin(n))
            .compressed()
            .run()
            .context("failed to generate SP1 compressed proof")?;
        if !mock_prover && verify_proofs {
            prover
                .verify(&compressed_proof, pk.verifying_key(), None)
                .context("failed to verify SP1 compressed proof")?;
        }
        Some(serialize_proof(&compressed_proof)?)
    } else {
        None
    };

    let groth16_proof = prover
        .prove(&pk, fibonacci_stdin(n))
        .groth16()
        .run()
        .context("failed to generate SP1 Groth16 proof")?;
    if !mock_prover && verify_proofs {
        prover
            .verify(&groth16_proof, pk.verifying_key(), None)
            .context("failed to verify SP1 Groth16 proof")?;
    }

    let public_values = if include_compressed_proof {
        public_values
    } else {
        groth16_proof.public_values.as_slice().to_vec()
    };

    let bundle = Sp1ProofBundle {
        public_values,
        stark_proof: compressed_proof.unwrap_or_default(),
        groth16_proof: serialize_proof(&groth16_proof)?,
    };

    let result = fibonacci_result(n).to_le_bytes().to_vec();

    Ok((result.clone(), bincode::serialize(&bundle)?, result))
}

/// Run the historical-average SP1 program.
///
/// `inputs` must be a bincode-encoded `Vec<u64>` (the lamport balances fetched
/// from the indexer by the coordinator).  The returned result is the 8-byte
/// little-endian average.
pub fn run_historical_avg_program(
    elf: &[u8],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    run_historical_avg_program_internal(elf, inputs, true, true)
}

#[cfg(test)]
pub(crate) fn run_historical_avg_program_groth16_only(
    elf: &[u8],
    inputs: &[u8],
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    run_historical_avg_program_internal(
        elf,
        inputs,
        false,
        should_verify_callback_proofs_locally(),
    )
}

pub(crate) fn run_historical_avg_program_groth16_proof(
    elf: &[u8],
    inputs: &[u8],
) -> anyhow::Result<Sp1Groth16ProofResult> {
    run_historical_avg_program_groth16_proof_internal(
        elf,
        inputs,
        should_verify_callback_proofs_locally(),
    )
}

fn run_historical_avg_program_groth16_proof_internal(
    elf: &[u8],
    inputs: &[u8],
    verify_proofs: bool,
) -> anyhow::Result<Sp1Groth16ProofResult> {
    let balances: Vec<u64> = bincode::deserialize(inputs)
        .context("failed to deserialize historical_avg inputs as Vec<u64>")?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    let prover = ProverClient::from_env();
    let pk = prover
        .setup(Elf::from(elf.to_vec()))
        .context("failed to set up SP1 proving key for historical_avg")?;

    let mut stdin = SP1Stdin::new();
    stdin.write(&balances);

    let groth16_proof = prover
        .prove(&pk, stdin)
        .groth16()
        .run()
        .context("failed to generate historical_avg SP1 Groth16 proof")?;
    if !mock_prover && verify_proofs {
        prover
            .verify(&groth16_proof, pk.verifying_key(), None)
            .context("failed to verify historical_avg SP1 Groth16 proof")?;
    }

    Ok(Sp1Groth16ProofResult {
        result: compute_historical_avg_result(&balances).to_le_bytes().to_vec(),
        proof: groth16_proof,
    })
}

fn run_historical_avg_program_internal(
    elf: &[u8],
    inputs: &[u8],
    include_compressed_proof: bool,
    verify_proofs: bool,
) -> anyhow::Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if !include_compressed_proof {
        let proof_result =
            run_historical_avg_program_groth16_proof_internal(elf, inputs, verify_proofs)?;
        let result = proof_result.result;
        let bundle = Sp1ProofBundle {
            public_values: proof_result.proof.public_values.as_slice().to_vec(),
            stark_proof: Vec::new(),
            groth16_proof: serialize_proof(&proof_result.proof)?,
        };

        return Ok((result.clone(), bincode::serialize(&bundle)?, result));
    }

    let balances: Vec<u64> = bincode::deserialize(inputs)
        .context("failed to deserialize historical_avg inputs as Vec<u64>")?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    if mock_prover && elf.is_empty() {
        let result_bytes = compute_historical_avg_result(&balances)
            .to_le_bytes()
            .to_vec();
        let bundle = Sp1ProofBundle {
            public_values: result_bytes.clone(),
            stark_proof: result_bytes.clone(),
            groth16_proof: result_bytes.clone(),
        };

        return Ok((
            result_bytes.clone(),
            bincode::serialize(&bundle)?,
            result_bytes,
        ));
    }

    let prover = ProverClient::from_env();
    let elf_obj = Elf::from(elf.to_vec());
    let pk = prover
        .setup(elf_obj.clone())
        .context("failed to set up SP1 proving key for historical_avg")?;

    let make_stdin = || {
        let mut stdin = SP1Stdin::new();
        stdin.write(&balances);
        stdin
    };

    let public_values = if include_compressed_proof {
        let (public_values, _report) = prover
            .execute(elf_obj.clone(), make_stdin())
            .run()
            .context("failed to execute historical_avg SP1 program")?;
        public_values.as_slice().to_vec()
    } else {
        Vec::new()
    };

    let compressed_proof = if include_compressed_proof {
        let compressed_proof = prover
            .prove(&pk, make_stdin())
            .compressed()
            .run()
            .context("failed to generate historical_avg SP1 compressed proof")?;
        if !mock_prover && verify_proofs {
            prover
                .verify(&compressed_proof, pk.verifying_key(), None)
                .context("failed to verify historical_avg SP1 compressed proof")?;
        }
        Some(serialize_proof(&compressed_proof)?)
    } else {
        None
    };

    let groth16_proof = prover
        .prove(&pk, make_stdin())
        .groth16()
        .run()
        .context("failed to generate historical_avg SP1 Groth16 proof")?;
    if !mock_prover && verify_proofs {
        prover
            .verify(&groth16_proof, pk.verifying_key(), None)
            .context("failed to verify historical_avg SP1 Groth16 proof")?;
    }

    let public_values = if include_compressed_proof {
        public_values
    } else {
        groth16_proof.public_values.as_slice().to_vec()
    };

    let bundle = Sp1ProofBundle {
        public_values,
        stark_proof: compressed_proof.unwrap_or_default(),
        groth16_proof: serialize_proof(&groth16_proof)?,
    };

    let result = compute_historical_avg_result(&balances);

    let result_bytes = result.to_le_bytes().to_vec();

    Ok((
        result_bytes.clone(),
        bincode::serialize(&bundle)?,
        result_bytes,
    ))
}

/// Compute the integer average of `balances` — mirrors the SP1 guest logic.
pub fn compute_historical_avg_result(balances: &[u64]) -> u64 {
    if balances.is_empty() {
        return 0;
    }

    let sum: u64 = balances.iter().fold(0u64, |accumulator, &value| {
        accumulator.saturating_add(value)
    });
    sum / balances.len() as u64
}

pub fn load_proof_bundle(stark_proof: &[u8]) -> anyhow::Result<Sp1ProofBundle> {
    bincode::deserialize(stark_proof).context("failed to decode SP1 proof bundle")
}

pub fn deserialize_proof(bytes: &[u8]) -> anyhow::Result<SP1ProofWithPublicValues> {
    bincode::deserialize(bytes)
        .or_else(|_| bincode::deserialize::<ProofFromNetwork>(bytes).map(Into::into))
        .context("failed to deserialize cached SP1 proof")
}

pub(crate) fn configure_prover_environment() {
    let prover = match std::env::var("SP1_PROVER") {
        Ok(prover) => prover,
        Err(_) => {
            let prover = if cfg!(any(test, feature = "mock")) {
                "mock"
            } else {
                "cpu"
            };
            std::env::set_var("SP1_PROVER", prover);
            prover.to_string()
        },
    };

    if matches!(prover.to_ascii_lowercase().as_str(), "cpu" | "cuda")
        && std::env::var("SHARD_SIZE").is_err()
    {
        std::env::set_var("SHARD_SIZE", DEFAULT_LOCAL_SP1_SHARD_SIZE);
    }
}

fn using_mock_prover() -> bool {
    std::env::var("SP1_PROVER")
        .map(|value| value.eq_ignore_ascii_case("mock"))
        .unwrap_or(false)
}

fn should_verify_callback_proofs_locally() -> bool {
    std::env::var("SONAR_VERIFY_CALLBACK_PROOFS_LOCALLY")
        .map(|value| {
            matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn fibonacci_stdin(n: u32) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    stdin.write(&n);
    stdin
}

fn fibonacci_result(n: u32) -> u32 {
    let mut a = 0u32;
    let mut b = 1u32;
    for _ in 0..n {
        let mut c = a + b;
        c %= 7919;
        a = b;
        b = c;
    }
    a
}

fn decode_fibonacci_input(inputs: &[u8]) -> anyhow::Result<u32> {
    let bytes: [u8; 4] = inputs
        .try_into()
        .map_err(|_| anyhow!("expected 4-byte little-endian fibonacci input"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn serialize_proof(proof: &SP1ProofWithPublicValues) -> anyhow::Result<Vec<u8>> {
    bincode::serialize(proof).context("failed to serialize SP1 proof")
}
