use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sp1_sdk::{
    blocking::{Elf, EnvProver, ProveRequest, Prover, ProverClient, SP1Stdin},
    ProofFromNetwork, ProvingKey, SP1ProofWithPublicValues,
};

// A smaller shard size reduces peak RAM per core-prover shard.  250 000 rows × ~16 field
// elements × 32 bytes ≈ 128 MB per shard.  100 000 rows halves that to ~50 MB.
const DEFAULT_LOCAL_SP1_SHARD_SIZE: &str = "100000";
const DEFAULT_SP1_WORKER_USE_FIXED_PK: &str = "true";
const DEFAULT_SP1_WORKER_VERIFY_INTERMEDIATES: &str = "false";

// Limits the Go runtime (gnark FFI) to one OS thread.  With the default of GOMAXPROCS=N_CPUS,
// gnark runs one goroutine per CPU core during the MSM/FFT steps, with each goroutine holding its
// own bucket-accumulator arrays.  Forcing GOMAXPROCS=1 serialises those goroutines so only one
// set of bucket arrays is live at a time — this cuts gnark's peak RAM by ~N_CPUS×.
//
// NOTE: the Go runtime reads GOMAXPROCS from the environment at the moment the first CGo function
// is invoked (i.e. when `groth16.Prove` is called for the first time in the process lifetime).
// Setting it here — before any SP1 prover call — is therefore sufficient.
const DEFAULT_GOMAXPROCS: &str = "1";

// Worker-count and buffer-size defaults targeting low-memory machines.
// These default to "1" so at most one shard or recursion proof is held in RAM at a time.
// Any of these can be overridden by the caller via the corresponding SP1_WORKER_* env var
// before `configure_prover_environment()` is called.
const DEFAULT_SP1_WORKER_NUM_CORE_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_CORE_BUFFER_SIZE: &str = "1";
const DEFAULT_SP1_WORKER_NUM_SETUP_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_SETUP_BUFFER_SIZE: &str = "1";
const DEFAULT_SP1_WORKER_NUM_RECURSION_PROVER_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE: &str = "1";
const DEFAULT_SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE: &str = "1";
const DEFAULT_SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE: &str = "1";
const DEFAULT_SP1_WORKER_NUM_SPLICING_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_SPLICING_BUFFER_SIZE: &str = "1";
const DEFAULT_SP1_WORKER_NUM_DEFERRED_WORKERS: &str = "1";
const DEFAULT_SP1_WORKER_DEFERRED_BUFFER_SIZE: &str = "1";

/// Minimum free RAM (bytes) required to attempt a real Groth16 proof locally.
/// gnark needs to hold the R1CS (~1.5 GB), proving key (~2.5 GB), and MSM working
/// memory (~0.5 GB with GOMAXPROCS=1) simultaneously → ~8 GB is a safe threshold.
#[cfg(test)]
pub(crate) const MIN_FREE_RAM_FOR_GROTH16: u64 = 8 * 1024 * 1024 * 1024;

type CachedBlockingProvingKey = <EnvProver as Prover>::ProvingKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProverRuntimeCacheKey {
    env: Vec<(String, String)>,
}

#[derive(Clone)]
struct CachedBlockingProver {
    prover: EnvProver,
    proving_keys: Arc<Mutex<HashMap<Vec<u8>, CachedBlockingProvingKey>>>,
}

static PROVER_CACHE: OnceLock<Mutex<HashMap<ProverRuntimeCacheKey, CachedBlockingProver>>> =
    OnceLock::new();

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
    run_sp1_program_groth16_proof_internal(elf, inputs, should_verify_callback_proofs_locally())
}

fn run_sp1_program_groth16_proof_internal(
    elf: &[u8],
    inputs: &[u8],
    verify_proofs: bool,
) -> anyhow::Result<Sp1Groth16ProofResult> {
    let n = decode_fibonacci_input(inputs)?;
    configure_prover_environment();
    let mock_prover = using_mock_prover();

    let cached = cached_blocking_prover()?;
    let pk = cached_proving_key(&cached, elf, "failed to set up SP1 proving key")?;
    let prover = &cached.prover;
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

    let cached = cached_blocking_prover()?;
    let pk = cached_proving_key(&cached, elf, "failed to set up SP1 proving key")?;
    let prover = &cached.prover;
    let elf = Elf::from(elf.to_vec());

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
    run_historical_avg_program_internal(elf, inputs, false, should_verify_callback_proofs_locally())
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

    let cached = cached_blocking_prover()?;
    let pk = cached_proving_key(
        &cached,
        elf,
        "failed to set up SP1 proving key for historical_avg",
    )?;
    let prover = &cached.prover;

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
        result: compute_historical_avg_result(&balances)
            .to_le_bytes()
            .to_vec(),
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

    let cached = cached_blocking_prover()?;
    let pk = cached_proving_key(
        &cached,
        elf,
        "failed to set up SP1 proving key for historical_avg",
    )?;
    let prover = &cached.prover;
    let elf_obj = Elf::from(elf.to_vec());

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

    if matches!(prover.to_ascii_lowercase().as_str(), "cpu" | "cuda") {
        set_env_default("SP1_WORKER_USE_FIXED_PK", DEFAULT_SP1_WORKER_USE_FIXED_PK);
        set_env_default(
            "SP1_WORKER_VERIFY_INTERMEDIATES",
            DEFAULT_SP1_WORKER_VERIFY_INTERMEDIATES,
        );
        // Limit the Go runtime used by the gnark Groth16 FFI to one OS thread.
        // gnark-crypto MSM creates one goroutine per CPU core; with GOMAXPROCS=1 those
        // goroutines serialise and only one set of bucket-accumulator arrays is live at once.
        // This must be set before the first CGo call (i.e. before groth16.Prove is invoked).
        set_env_default("GOMAXPROCS", DEFAULT_GOMAXPROCS);
        // Worker concurrency / buffer defaults: keep at most 1 shard or recursion proof
        // live in RAM simultaneously so the prover fits inside small-machine memory budgets.
        // Any var already set in the environment (e.g. by an operator) is left untouched.
        set_env_default(
            "SP1_WORKER_NUM_CORE_WORKERS",
            DEFAULT_SP1_WORKER_NUM_CORE_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_CORE_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_CORE_BUFFER_SIZE,
        );
        set_env_default(
            "SP1_WORKER_NUM_SETUP_WORKERS",
            DEFAULT_SP1_WORKER_NUM_SETUP_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_SETUP_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_SETUP_BUFFER_SIZE,
        );
        set_env_default(
            "SP1_WORKER_NUM_RECURSION_PROVER_WORKERS",
            DEFAULT_SP1_WORKER_NUM_RECURSION_PROVER_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE,
        );
        set_env_default(
            "SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS",
            DEFAULT_SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE,
        );
        set_env_default(
            "SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS",
            DEFAULT_SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE,
        );
        set_env_default(
            "SP1_WORKER_NUM_SPLICING_WORKERS",
            DEFAULT_SP1_WORKER_NUM_SPLICING_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_SPLICING_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_SPLICING_BUFFER_SIZE,
        );
        set_env_default(
            "SP1_WORKER_NUM_DEFERRED_WORKERS",
            DEFAULT_SP1_WORKER_NUM_DEFERRED_WORKERS,
        );
        set_env_default(
            "SP1_WORKER_DEFERRED_BUFFER_SIZE",
            DEFAULT_SP1_WORKER_DEFERRED_BUFFER_SIZE,
        );
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
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
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

fn cached_blocking_prover() -> anyhow::Result<CachedBlockingProver> {
    let cache = PROVER_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .map_err(|_| anyhow!("SP1 prover cache mutex poisoned"))?;

    let runtime = current_prover_runtime_key();
    let cached = cache
        .entry(runtime)
        .or_insert_with(|| CachedBlockingProver {
            prover: ProverClient::from_env(),
            proving_keys: Arc::new(Mutex::new(HashMap::new())),
        })
        .clone();

    Ok(cached)
}

fn cached_proving_key(
    cached: &CachedBlockingProver,
    elf: &[u8],
    setup_error_context: &'static str,
) -> anyhow::Result<CachedBlockingProvingKey> {
    let mut keys = cached
        .proving_keys
        .lock()
        .map_err(|_| anyhow!("SP1 proving key cache mutex poisoned"))?;

    if let Some(pk) = keys.get(elf) {
        return Ok(pk.clone());
    }

    let pk = cached
        .prover
        .setup(Elf::from(elf.to_vec()))
        .context(setup_error_context)?;
    keys.insert(elf.to_vec(), pk.clone());
    Ok(pk)
}

fn current_prover_runtime_key() -> ProverRuntimeCacheKey {
    let mut env = std::env::vars()
        .filter(|(name, _)| is_prover_runtime_env(name))
        .collect::<Vec<_>>();
    env.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    ProverRuntimeCacheKey { env }
}

fn is_prover_runtime_env(name: &str) -> bool {
    name == "SP1_PROVER"
        || name == "SHARD_SIZE"
        || name == "RAYON_NUM_THREADS"
        || name.starts_with("SP1_WORKER_")
}

fn set_env_default(name: &str, value: &str) {
    if std::env::var(name).is_err() {
        std::env::set_var(name, value);
    }
}

fn serialize_proof(proof: &SP1ProofWithPublicValues) -> anyhow::Result<Vec<u8>> {
    bincode::serialize(proof).context("failed to serialize SP1 proof")
}

/// Returns the number of bytes of available (free + reclaimable) RAM on Linux by reading
/// `MemAvailable` from `/proc/meminfo`.  Returns `None` on non-Linux platforms or on parse
/// failure.
#[cfg(test)]
pub(crate) fn available_ram_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{
        configure_prover_environment, DEFAULT_GOMAXPROCS, DEFAULT_LOCAL_SP1_SHARD_SIZE,
        DEFAULT_SP1_WORKER_CORE_BUFFER_SIZE, DEFAULT_SP1_WORKER_DEFERRED_BUFFER_SIZE,
        DEFAULT_SP1_WORKER_NUM_CORE_WORKERS, DEFAULT_SP1_WORKER_NUM_DEFERRED_WORKERS,
        DEFAULT_SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS,
        DEFAULT_SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS,
        DEFAULT_SP1_WORKER_NUM_RECURSION_PROVER_WORKERS,
        DEFAULT_SP1_WORKER_NUM_SETUP_WORKERS, DEFAULT_SP1_WORKER_NUM_SPLICING_WORKERS,
        DEFAULT_SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE,
        DEFAULT_SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE,
        DEFAULT_SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE, DEFAULT_SP1_WORKER_SETUP_BUFFER_SIZE,
        DEFAULT_SP1_WORKER_SPLICING_BUFFER_SIZE, DEFAULT_SP1_WORKER_USE_FIXED_PK,
        DEFAULT_SP1_WORKER_VERIFY_INTERMEDIATES,
    };

    static SP1_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// All env vars touched by configure_prover_environment, so we can save/restore them.
    const WORKER_VARS: &[&str] = &[
        "SHARD_SIZE",
        "GOMAXPROCS",
        "SP1_WORKER_USE_FIXED_PK",
        "SP1_WORKER_VERIFY_INTERMEDIATES",
        "SP1_WORKER_NUM_CORE_WORKERS",
        "SP1_WORKER_CORE_BUFFER_SIZE",
        "SP1_WORKER_NUM_SETUP_WORKERS",
        "SP1_WORKER_SETUP_BUFFER_SIZE",
        "SP1_WORKER_NUM_RECURSION_PROVER_WORKERS",
        "SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE",
        "SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS",
        "SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE",
        "SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS",
        "SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE",
        "SP1_WORKER_NUM_SPLICING_WORKERS",
        "SP1_WORKER_SPLICING_BUFFER_SIZE",
        "SP1_WORKER_NUM_DEFERRED_WORKERS",
        "SP1_WORKER_DEFERRED_BUFFER_SIZE",
    ];

    #[test]
    fn configure_prover_environment_sets_local_cpu_defaults() {
        let _guard = SP1_ENV_LOCK.lock().expect("SP1 env lock should not be poisoned");
        let previous_prover = std::env::var("SP1_PROVER").ok();
        let previous: Vec<_> = WORKER_VARS.iter().map(|k| std::env::var(k).ok()).collect();

        std::env::set_var("SP1_PROVER", "cpu");
        for k in WORKER_VARS {
            std::env::remove_var(k);
        }

        configure_prover_environment();

        assert_eq!(
            std::env::var("SHARD_SIZE").as_deref(),
            Ok(DEFAULT_LOCAL_SP1_SHARD_SIZE)
        );
        assert_eq!(std::env::var("GOMAXPROCS").as_deref(), Ok(DEFAULT_GOMAXPROCS));
        assert_eq!(
            std::env::var("SP1_WORKER_USE_FIXED_PK").as_deref(),
            Ok(DEFAULT_SP1_WORKER_USE_FIXED_PK)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_VERIFY_INTERMEDIATES").as_deref(),
            Ok(DEFAULT_SP1_WORKER_VERIFY_INTERMEDIATES)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_CORE_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_CORE_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_CORE_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_CORE_BUFFER_SIZE)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_SETUP_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_SETUP_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_SETUP_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_SETUP_BUFFER_SIZE)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_RECURSION_PROVER_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_RECURSION_PROVER_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_RECURSION_PROVER_BUFFER_SIZE)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_RECURSION_EXECUTOR_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_RECURSION_EXECUTOR_BUFFER_SIZE)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_PREPARE_REDUCE_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_PREPARE_REDUCE_BUFFER_SIZE)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_SPLICING_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_SPLICING_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_SPLICING_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_SPLICING_BUFFER_SIZE)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_NUM_DEFERRED_WORKERS").as_deref(),
            Ok(DEFAULT_SP1_WORKER_NUM_DEFERRED_WORKERS)
        );
        assert_eq!(
            std::env::var("SP1_WORKER_DEFERRED_BUFFER_SIZE").as_deref(),
            Ok(DEFAULT_SP1_WORKER_DEFERRED_BUFFER_SIZE)
        );

        restore_env_var("SP1_PROVER", previous_prover);
        for (k, v) in WORKER_VARS.iter().zip(previous) {
            restore_env_var(k, v);
        }
    }

    #[test]
    fn configure_prover_environment_leaves_mock_worker_defaults_unset() {
        let _guard = SP1_ENV_LOCK.lock().expect("SP1 env lock should not be poisoned");
        let previous_prover = std::env::var("SP1_PROVER").ok();
        let previous: Vec<_> = WORKER_VARS.iter().map(|k| std::env::var(k).ok()).collect();

        std::env::set_var("SP1_PROVER", "mock");
        for k in WORKER_VARS {
            std::env::remove_var(k);
        }

        configure_prover_environment();

        for k in WORKER_VARS {
            assert!(
                std::env::var(k).is_err(),
                "expected {k} to be unset for mock prover"
            );
        }

        restore_env_var("SP1_PROVER", previous_prover);
        for (k, v) in WORKER_VARS.iter().zip(previous) {
            restore_env_var(k, v);
        }
    }

    fn restore_env_var(name: &str, value: Option<String>) {
        if let Some(value) = value {
            std::env::set_var(name, value);
        } else {
            std::env::remove_var(name);
        }
    }
}
