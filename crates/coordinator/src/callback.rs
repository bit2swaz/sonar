//! Callback worker — consumes [`ProverResponse`]s from Redis and submits the
//! Sonar `callback` instruction on-chain.
//!
//! Flow:
//! 1. BLPOP from `sonar:responses`.
//! 2. Derive the `RequestMetadata` and `ResultAccount` PDAs.
//! 3. Fetch `RequestMetadata` to learn the `callback_program` address.
//! 4. Build and sign the `callback` instruction.
//! 5. Send and confirm the transaction (with retries).

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context as _;
use futures_util::{future::BoxFuture, FutureExt};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::{client_error::Result as ClientResult, rpc_response::RpcPrioritizationFee};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    hash::hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use tracing::{error, info, warn};

use sonar_common::types::{CallbackAccountMeta as CommonCallbackAccountMeta, ProverResponse};

use crate::{dispatcher, listener};

const GROTH16_PROOF_A_BYTES: usize = 64;
const GROTH16_PROOF_B_BYTES: usize = 128;
const GROTH16_PROOF_C_BYTES: usize = 64;
const GROTH16_PROOF_BYTES: usize =
    GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES + GROTH16_PROOF_C_BYTES;
const GROTH16_PUBLIC_INPUT_BYTES: usize = 32;
const PRIORITY_FEE_PERCENTILE: usize = 75;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedCallbackPayload {
    proof: Vec<u8>,
    public_inputs: Vec<Vec<u8>>,
    flattened_public_inputs: Vec<u8>,
    result: Vec<u8>,
    proof_a: Option<[u8; GROTH16_PROOF_A_BYTES]>,
    proof_b: Option<[u8; GROTH16_PROOF_B_BYTES]>,
    proof_c: Option<[u8; GROTH16_PROOF_C_BYTES]>,
}

fn normalize_callback_payload(
    response: &ProverResponse,
) -> anyhow::Result<NormalizedCallbackPayload> {
    if response.proof.len() == GROTH16_PROOF_BYTES {
        let proof_a: [u8; GROTH16_PROOF_A_BYTES] = response.proof[..GROTH16_PROOF_A_BYTES]
            .try_into()
            .context("invalid Groth16 proof_a segment length")?;
        let proof_b: [u8; GROTH16_PROOF_B_BYTES] = response.proof
            [GROTH16_PROOF_A_BYTES..GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES]
            .try_into()
            .context("invalid Groth16 proof_b segment length")?;
        let proof_c: [u8; GROTH16_PROOF_C_BYTES] = response.proof
            [GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES..]
            .try_into()
            .context("invalid Groth16 proof_c segment length")?;

        let flattened_public_inputs =
            flatten_public_inputs_exact(&response.public_inputs, GROTH16_PUBLIC_INPUT_BYTES)?;

        return Ok(NormalizedCallbackPayload {
            proof: [&proof_a[..], &proof_b[..], &proof_c[..]].concat(),
            public_inputs: response.public_inputs.clone(),
            flattened_public_inputs,
            result: response.result.clone(),
            proof_a: Some(proof_a),
            proof_b: Some(proof_b),
            proof_c: Some(proof_c),
        });
    }

    if response.proof.is_empty() {
        anyhow::bail!("callback response proof is empty");
    }

    let flattened_public_inputs = flatten_public_inputs_any(&response.public_inputs)?;

    Ok(NormalizedCallbackPayload {
        proof: response.proof.clone(),
        public_inputs: response.public_inputs.clone(),
        flattened_public_inputs,
        result: response.result.clone(),
        proof_a: None,
        proof_b: None,
        proof_c: None,
    })
}

fn flatten_public_inputs_exact(public_inputs: &[Vec<u8>], width: usize) -> anyhow::Result<Vec<u8>> {
    let mut flattened = Vec::with_capacity(public_inputs.len() * width);
    for (index, input) in public_inputs.iter().enumerate() {
        if input.len() != width {
            anyhow::bail!(
                "public input {index} has length {}; expected {width}",
                input.len()
            );
        }
        flattened.extend_from_slice(input);
    }
    Ok(flattened)
}

fn flatten_public_inputs_any(public_inputs: &[Vec<u8>]) -> anyhow::Result<Vec<u8>> {
    if public_inputs.is_empty() {
        anyhow::bail!("callback response has no public inputs");
    }

    let total_len = public_inputs.iter().map(Vec::len).sum();
    let mut flattened = Vec::with_capacity(total_len);
    for input in public_inputs {
        flattened.extend_from_slice(input);
    }
    Ok(flattened)
}

// ---------------------------------------------------------------------------
// Discriminator helpers
// ---------------------------------------------------------------------------

/// Anchor instruction discriminator: SHA-256(`"global:callback"`)[..8].
pub fn callback_discriminator() -> [u8; 8] {
    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&hash(b"global:callback").to_bytes()[..8]);
    discriminator
}

// ---------------------------------------------------------------------------
// Borsh-style encoding (manual — no extra dep)
// ---------------------------------------------------------------------------

/// Encode `CallbackParams { proof, public_inputs, result }` in Anchor/borsh
/// wire format (length-prefixed vectors, all lengths as `u32 LE`).
pub fn encode_callback_params(proof: &[u8], public_inputs: &[Vec<u8>], result: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();

    // proof: Vec<u8>
    buf.extend_from_slice(&(proof.len() as u32).to_le_bytes());
    buf.extend_from_slice(proof);

    // public_inputs: Vec<Vec<u8>>
    buf.extend_from_slice(&(public_inputs.len() as u32).to_le_bytes());
    for pi in public_inputs {
        buf.extend_from_slice(&(pi.len() as u32).to_le_bytes());
        buf.extend_from_slice(pi);
    }

    // result: Vec<u8>
    buf.extend_from_slice(&(result.len() as u32).to_le_bytes());
    buf.extend_from_slice(result);

    buf
}

/// Build the instruction data bytes: discriminator || encoded params.
pub fn build_callback_instruction_data(
    proof: &[u8],
    public_inputs: &[Vec<u8>],
    result: &[u8],
) -> Vec<u8> {
    let mut data = callback_discriminator().to_vec();
    data.extend(encode_callback_params(proof, public_inputs, result));
    data
}

// ---------------------------------------------------------------------------
// Instruction builder
// ---------------------------------------------------------------------------

/// Construct the Sonar `callback` [`Instruction`].
///
/// Accounts (in Anchor order):
/// 0. `request_metadata` — mutable, not signer (PDA)
/// 1. `result_account`   — mutable, not signer (PDA)
/// 2. `verifier_registry` — read-only, not signer (PDA)
/// 3. `prover`            — mutable, signer (coordinator keypair)
/// 4. `payer`             — mutable, not signer (original request payer)
/// 5. `callback_program`  — not mutable, not signer
/// 6+. consumer callback remaining accounts, in the order recorded by the request
#[allow(clippy::too_many_arguments)]
pub fn build_callback_instruction(
    program_id: Pubkey,
    request_id: &[u8; 32],
    computation_id: &[u8; 32],
    prover_pubkey: Pubkey,
    payer_pubkey: Pubkey,
    callback_program: Pubkey,
    callback_accounts: &[CommonCallbackAccountMeta],
    proof: &[u8],
    public_inputs: &[Vec<u8>],
    result: &[u8],
) -> (Instruction, Pubkey, Pubkey) {
    let (request_metadata_pda, _) =
        Pubkey::find_program_address(&[b"request", request_id.as_ref()], &program_id);
    let (result_account_pda, _) =
        Pubkey::find_program_address(&[b"result", request_id.as_ref()], &program_id);
    let (verifier_registry_pda, _) =
        Pubkey::find_program_address(&[b"verifier", computation_id.as_ref()], &program_id);

    let mut accounts = vec![
        AccountMeta::new(request_metadata_pda, false),
        AccountMeta::new(result_account_pda, false),
        AccountMeta::new_readonly(verifier_registry_pda, false),
        AccountMeta::new(prover_pubkey, true),
        AccountMeta::new(payer_pubkey, false),
        AccountMeta::new_readonly(callback_program, false),
    ];

    for callback_account in callback_accounts {
        let pubkey = Pubkey::new_from_array(*callback_account.pubkey.as_bytes());
        accounts.push(if callback_account.is_writable {
            AccountMeta::new(pubkey, false)
        } else {
            AccountMeta::new_readonly(pubkey, false)
        });
    }

    let data = build_callback_instruction_data(proof, public_inputs, result);

    let ix = Instruction {
        program_id,
        accounts,
        data,
    };

    (ix, request_metadata_pda, result_account_pda)
}

trait PrioritizationFeeClient {
    fn get_recent_prioritization_fees<'a>(
        &'a self,
        writable_accounts: &'a [Pubkey],
    ) -> BoxFuture<'a, ClientResult<Vec<RpcPrioritizationFee>>>;
}

impl PrioritizationFeeClient for RpcClient {
    fn get_recent_prioritization_fees<'a>(
        &'a self,
        writable_accounts: &'a [Pubkey],
    ) -> BoxFuture<'a, ClientResult<Vec<RpcPrioritizationFee>>> {
        RpcClient::get_recent_prioritization_fees(self, writable_accounts).boxed()
    }
}

fn estimate_priority_fee_micro_lamports(samples: &[RpcPrioritizationFee]) -> u64 {
    let mut fees = samples
        .iter()
        .map(|sample| sample.prioritization_fee)
        .filter(|fee| *fee > 0)
        .collect::<Vec<_>>();

    if fees.is_empty() {
        fees = samples
            .iter()
            .map(|sample| sample.prioritization_fee)
            .collect::<Vec<_>>();
    }

    if fees.is_empty() {
        return 0;
    }

    fees.sort_unstable();
    let rank = (fees.len() * PRIORITY_FEE_PERCENTILE).div_ceil(100);
    let index = rank.saturating_sub(1).min(fees.len() - 1);
    fees[index]
}

async fn fetch_priority_fee_estimate<C>(
    client: &C,
    writable_accounts: &[Pubkey],
) -> anyhow::Result<u64>
where
    C: PrioritizationFeeClient + ?Sized,
{
    let fees = client
        .get_recent_prioritization_fees(writable_accounts)
        .await
        .context("getRecentPrioritizationFees")?;

    Ok(estimate_priority_fee_micro_lamports(&fees))
}

fn build_callback_transaction_instructions(
    callback_instruction: Instruction,
    priority_fee_micro_lamports: u64,
) -> Vec<Instruction> {
    vec![
        ComputeBudgetInstruction::set_compute_unit_price(priority_fee_micro_lamports),
        callback_instruction,
    ]
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Runtime configuration for the callback worker task.
pub struct CallbackConfig {
    pub redis_url: String,
    pub responses_queue: String,
    pub rpc_url: String,
    pub program_id_str: String,
    /// Coordinator / prover keypair used to sign callback transactions.
    pub prover_keypair: Arc<Keypair>,
    /// Seconds to wait in each BLPOP call before looping.
    pub blpop_timeout_secs: f64,
    /// Maximum send-and-confirm retries per response.
    pub max_retries: u32,
}

// ---------------------------------------------------------------------------
// Main callback worker task
// ---------------------------------------------------------------------------

/// Pop [`ProverResponse`]s from Redis and submit callback transactions.
/// Returns when `shutdown` receives `true`.
pub async fn run_callback_worker(
    config: CallbackConfig,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let program_id = Pubkey::from_str(&config.program_id_str).context("parse program_id")?;

    let rpc = RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed());

    let redis_client = redis::Client::open(config.redis_url.as_str()).context("redis client")?;
    let mut redis_conn = redis_client
        .get_multiplexed_async_connection()
        .await
        .context("redis connect")?;

    info!("Callback worker started");

    loop {
        // Check shutdown before blocking on Redis.
        if *shutdown.borrow() {
            info!("Callback worker shutting down");
            break;
        }

        let response = match dispatcher::pop_response(
            &mut redis_conn,
            &config.responses_queue,
            config.blpop_timeout_secs,
        )
        .await
        {
            Ok(Some(r)) => r,
            Ok(None) => continue, // timeout, loop again
            Err(e) => {
                warn!("pop_response error: {e:#}");
                continue;
            },
        };

        info!(
            "Processing response for request {:?}",
            hex_encode(&response.request_id)
        );

        if let Err(e) = process_response(
            &rpc,
            &program_id,
            &config.prover_keypair,
            config.max_retries,
            &response,
        )
        .await
        {
            error!("process_response failed: {e:#}");
        }
    }

    Ok(())
}

/// Fetch the `RequestMetadata` account, build the callback instruction, and
/// submit the transaction with `max_retries` attempts.
async fn process_response(
    rpc: &RpcClient,
    program_id: &Pubkey,
    keypair: &Keypair,
    max_retries: u32,
    response: &ProverResponse,
) -> anyhow::Result<()> {
    // Derive request metadata PDA.
    let (pda, _) =
        Pubkey::find_program_address(&[b"request", response.request_id.as_ref()], program_id);

    // Fetch and decode account to get callback_program.
    let account_data = rpc
        .get_account_data(&pda)
        .await
        .context("fetch RequestMetadata")?;
    let meta =
        listener::decode_request_metadata(&account_data).context("decode RequestMetadata")?;

    let payload = normalize_callback_payload(response).context("normalize callback payload")?;
    let proof_len = payload.proof.len();
    let flattened_public_inputs_len = payload.flattened_public_inputs.len();

    let callback_program = Pubkey::new_from_array(meta.callback_program);
    let payer = Pubkey::new_from_array(meta.payer);

    // Build instruction with prover-provided public inputs.
    let (ix, _, _) = build_callback_instruction(
        *program_id,
        &response.request_id,
        &meta.computation_id,
        keypair.pubkey(),
        payer,
        callback_program,
        &response.callback_accounts,
        &payload.proof,
        &payload.public_inputs,
        &payload.result,
    );

    let writable_accounts = ix
        .accounts
        .iter()
        .filter(|account| account.is_writable)
        .map(|account| account.pubkey)
        .collect::<Vec<_>>();

    let priority_fee_micro_lamports =
        match fetch_priority_fee_estimate(rpc, &writable_accounts).await {
            Ok(priority_fee_micro_lamports) => priority_fee_micro_lamports,
            Err(error) => {
                warn!(
                    error = ?error,
                    "priority fee estimation failed; falling back to zero micro-lamports"
                );
                0
            },
        };

    let instructions = build_callback_transaction_instructions(ix, priority_fee_micro_lamports);

    info!(
        proof_len,
        flattened_public_inputs_len,
        priority_fee_micro_lamports,
        callback_accounts = response.callback_accounts.len(),
        computation_id = %hex_encode(&meta.computation_id),
        "formatted callback payload"
    );

    // Send with retries.
    for attempt in 0..=max_retries {
        let blockhash = rpc
            .get_latest_blockhash()
            .await
            .context("get_latest_blockhash")?;

        let tx = Transaction::new_signed_with_payer(
            &instructions,
            Some(&keypair.pubkey()),
            &[keypair],
            blockhash,
        );

        match rpc.send_and_confirm_transaction(&tx).await {
            Ok(sig) => {
                info!("Callback confirmed: {sig}");
                return Ok(());
            },
            Err(e) if attempt < max_retries => {
                warn!("Callback attempt {attempt} failed: {e:#} — retrying");
            },
            Err(e) => {
                return Err(e).context("send_and_confirm_transaction");
            },
        }
    }

    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // --- callback_discriminator ---

    #[test]
    fn discriminator_is_8_bytes() {
        let d = callback_discriminator();
        assert_eq!(d.len(), 8);
    }

    #[test]
    fn discriminator_is_deterministic() {
        assert_eq!(callback_discriminator(), callback_discriminator());
    }

    // --- encode_callback_params ---

    #[test]
    fn encode_empty_params() {
        let enc = encode_callback_params(&[], &[], &[]);
        // 3 empty Vec<u8> → 3 × 4 bytes of length prefix = 12 bytes
        assert_eq!(enc.len(), 12);
        // All lengths are zero.
        assert!(enc.iter().all(|&b| b == 0));
    }

    #[test]
    fn encode_params_with_data() {
        let proof = vec![0xABu8; 8];
        let public_inputs = vec![vec![1u8, 2u8], vec![3u8, 4u8]];
        let result = vec![0xFFu8; 4];

        let enc = encode_callback_params(&proof, &public_inputs, &result);

        let mut cursor = 0usize;

        // proof length
        let plen = u32::from_le_bytes(enc[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        assert_eq!(plen, 8);
        cursor += plen;

        // public_inputs outer length
        let outer_len = u32::from_le_bytes(enc[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        assert_eq!(outer_len, 2);

        for &expected_inner_len in &[2usize, 2usize] {
            let inner_len =
                u32::from_le_bytes(enc[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
            assert_eq!(inner_len, expected_inner_len);
            cursor += inner_len;
        }

        // result length
        let rlen = u32::from_le_bytes(enc[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        assert_eq!(rlen, 4);
        cursor += rlen;

        assert_eq!(cursor, enc.len());
    }

    // --- build_callback_instruction_data ---

    #[test]
    fn instruction_data_starts_with_discriminator() {
        let data = build_callback_instruction_data(&[], &[], &[]);
        let disc = callback_discriminator();
        assert_eq!(&data[..8], disc.as_ref());
    }

    #[test]
    fn instruction_data_ends_with_encoded_params() {
        let proof = vec![1u8, 2, 3];
        let data = build_callback_instruction_data(&proof, &[], &[]);
        // After 8-byte discriminator: 4 bytes proof len + 3 bytes proof data
        //                            + 4 bytes empty public_inputs len
        //                            + 4 bytes empty result len
        assert_eq!(data.len(), 8 + 4 + 3 + 4 + 4);
    }

    // --- build_callback_instruction ---

    #[test]
    fn instruction_has_six_accounts() {
        let program_id = Pubkey::new_unique();
        let request_id = [0u8; 32];
        let computation_id = [1u8; 32];
        let prover = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let cb_prog = Pubkey::new_unique();

        let (ix, _, _) = build_callback_instruction(
            program_id,
            &request_id,
            &computation_id,
            prover,
            payer,
            cb_prog,
            &[],
            &[],
            &[],
            &[],
        );

        assert_eq!(ix.accounts.len(), 6);
        // Account 2 (verifier registry) must be read-only and not signer.
        assert!(!ix.accounts[2].is_writable && !ix.accounts[2].is_signer);
        // Account 3 (prover) must be a signer.
        assert!(ix.accounts[3].is_signer);
        // Account 4 (payer) must be writable and not a signer.
        assert!(ix.accounts[4].is_writable && !ix.accounts[4].is_signer);
        // Accounts 0 and 1 must be mutable but not signers.
        assert!(ix.accounts[0].is_writable && !ix.accounts[0].is_signer);
        assert!(ix.accounts[1].is_writable && !ix.accounts[1].is_signer);
        // Account 5 (callback_program) must be read-only.
        assert!(!ix.accounts[5].is_writable && !ix.accounts[5].is_signer);
    }

    #[test]
    fn pdas_are_derived_from_request_id() {
        let program_id = Pubkey::new_unique();
        let request_id = [7u8; 32];
        let computation_id = [8u8; 32];
        let prover = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let cb_prog = Pubkey::new_unique();

        let (ix, req_pda, res_pda) = build_callback_instruction(
            program_id,
            &request_id,
            &computation_id,
            prover,
            payer,
            cb_prog,
            &[],
            &[],
            &[],
            &[],
        );

        let (expected_req, _) =
            Pubkey::find_program_address(&[b"request", &request_id], &program_id);
        let (expected_res, _) =
            Pubkey::find_program_address(&[b"result", &request_id], &program_id);
        let (expected_verifier, _) =
            Pubkey::find_program_address(&[b"verifier", &computation_id], &program_id);

        assert_eq!(req_pda, expected_req);
        assert_eq!(res_pda, expected_res);
        assert_eq!(ix.accounts[2].pubkey, expected_verifier);
    }

    #[test]
    fn estimate_priority_fee_uses_nonzero_p75_sample() {
        let samples = vec![
            RpcPrioritizationFee {
                slot: 100,
                prioritization_fee: 0,
            },
            RpcPrioritizationFee {
                slot: 101,
                prioritization_fee: 1_000,
            },
            RpcPrioritizationFee {
                slot: 102,
                prioritization_fee: 2_000,
            },
            RpcPrioritizationFee {
                slot: 103,
                prioritization_fee: 3_000,
            },
        ];

        assert_eq!(estimate_priority_fee_micro_lamports(&samples), 3_000);
    }

    #[test]
    fn instruction_appends_callback_accounts_after_fixed_accounts() {
        let program_id = Pubkey::new_unique();
        let callback_accounts = vec![
            CommonCallbackAccountMeta {
                pubkey: sonar_common::types::Pubkey::new([0x11; 32]),
                is_writable: true,
            },
            CommonCallbackAccountMeta {
                pubkey: sonar_common::types::Pubkey::new([0x22; 32]),
                is_writable: false,
            },
        ];

        let (ix, _, _) = build_callback_instruction(
            program_id,
            &[7u8; 32],
            &[8u8; 32],
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            &callback_accounts,
            &[],
            &[],
            &[],
        );

        assert_eq!(ix.accounts.len(), 8);
        assert_eq!(ix.accounts[6].pubkey, Pubkey::new_from_array([0x11; 32]));
        assert!(ix.accounts[6].is_writable);
        assert_eq!(ix.accounts[7].pubkey, Pubkey::new_from_array([0x22; 32]));
        assert!(!ix.accounts[7].is_writable);
    }

    #[test]
    fn callback_transaction_instructions_prepend_compute_budget_price() {
        let callback_instruction = Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta::new(Pubkey::new_unique(), false)],
            data: vec![1, 2, 3],
        };

        let instructions =
            build_callback_transaction_instructions(callback_instruction.clone(), 42_000);

        assert_eq!(instructions.len(), 2);
        assert_eq!(
            instructions[0],
            ComputeBudgetInstruction::set_compute_unit_price(42_000)
        );
        assert_eq!(instructions[1], callback_instruction);
    }

    struct MockPrioritizationFeeClient {
        fees: Vec<RpcPrioritizationFee>,
        writable_accounts: Mutex<Vec<Pubkey>>,
    }

    impl PrioritizationFeeClient for MockPrioritizationFeeClient {
        fn get_recent_prioritization_fees<'a>(
            &'a self,
            writable_accounts: &'a [Pubkey],
        ) -> BoxFuture<'a, ClientResult<Vec<RpcPrioritizationFee>>> {
            *self
                .writable_accounts
                .lock()
                .expect("lock writable accounts") = writable_accounts.to_vec();
            futures_util::future::ready(Ok(self.fees.clone())).boxed()
        }
    }

    #[tokio::test]
    async fn fetch_priority_fee_estimate_uses_rpc_response_for_writable_accounts() {
        let writable_accounts = vec![Pubkey::new_unique(), Pubkey::new_unique()];
        let client = MockPrioritizationFeeClient {
            fees: vec![
                RpcPrioritizationFee {
                    slot: 1,
                    prioritization_fee: 500,
                },
                RpcPrioritizationFee {
                    slot: 2,
                    prioritization_fee: 2_000,
                },
                RpcPrioritizationFee {
                    slot: 3,
                    prioritization_fee: 5_000,
                },
                RpcPrioritizationFee {
                    slot: 4,
                    prioritization_fee: 9_000,
                },
            ],
            writable_accounts: Mutex::new(Vec::new()),
        };

        let estimate = fetch_priority_fee_estimate(&client, &writable_accounts)
            .await
            .expect("priority fee estimate should succeed");

        assert_eq!(estimate, 5_000);
        assert_eq!(
            *client
                .writable_accounts
                .lock()
                .expect("lock writable accounts"),
            writable_accounts
        );
    }

    #[test]
    fn normalize_groth16_payload_splits_proof_and_flattens_public_inputs() {
        let proof: Vec<u8> = (0..GROTH16_PROOF_BYTES as u16)
            .map(|value| value as u8)
            .collect();
        let public_inputs = vec![vec![0x11; 32], vec![0x22; 32]];
        let response = ProverResponse {
            request_id: [9u8; 32],
            result: vec![0xAA, 0xBB],
            proof: proof.clone(),
            public_inputs: public_inputs.clone(),
            gas_used: 123,
            callback_accounts: vec![],
        };

        let payload = normalize_callback_payload(&response).expect("payload should normalize");

        assert_eq!(payload.proof, proof);
        assert_eq!(payload.proof_a.expect("proof_a"), proof[..64]);
        assert_eq!(payload.proof_b.expect("proof_b"), proof[64..192]);
        assert_eq!(payload.proof_c.expect("proof_c"), proof[192..256]);
        assert_eq!(
            payload.flattened_public_inputs,
            [vec![0x11; 32], vec![0x22; 32]].concat()
        );
        assert_eq!(payload.public_inputs, public_inputs);
    }

    #[test]
    fn normalize_groth16_payload_rejects_wrong_public_input_size() {
        let response = ProverResponse {
            request_id: [1u8; 32],
            result: vec![7u8; 4],
            proof: vec![3u8; GROTH16_PROOF_BYTES],
            public_inputs: vec![vec![9u8; 31]],
            gas_used: 1,
            callback_accounts: vec![],
        };

        let error = normalize_callback_payload(&response).expect_err("payload should fail");
        assert!(error
            .to_string()
            .contains("public input 0 has length 31; expected 32"));
    }

    #[test]
    fn normalize_legacy_payload_preserves_existing_mvp_shape() {
        let response = ProverResponse {
            request_id: [2u8; 32],
            result: vec![5u8; 8],
            proof: b"historical-avg-mock-proof".to_vec(),
            public_inputs: vec![vec![4u8; 8]],
            gas_used: 42,
            callback_accounts: vec![],
        };

        let payload = normalize_callback_payload(&response).expect("legacy payload should pass");

        assert_eq!(payload.proof, response.proof);
        assert_eq!(payload.public_inputs, response.public_inputs);
        assert_eq!(payload.flattened_public_inputs, vec![4u8; 8]);
        assert!(payload.proof_a.is_none());
        assert!(payload.proof_b.is_none());
        assert!(payload.proof_c.is_none());
    }

    #[test]
    fn normalize_payload_rejects_empty_legacy_proof() {
        let response = ProverResponse {
            request_id: [3u8; 32],
            result: vec![],
            proof: vec![],
            public_inputs: vec![vec![1u8; 8]],
            gas_used: 0,
            callback_accounts: vec![],
        };

        let error = normalize_callback_payload(&response).expect_err("empty proof should fail");
        assert!(error
            .to_string()
            .contains("callback response proof is empty"));
    }
}
