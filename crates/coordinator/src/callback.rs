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
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    hash::hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use tracing::{error, info, warn};

use sonar_common::types::ProverResponse;

use crate::{dispatcher, listener};

// ---------------------------------------------------------------------------
// Discriminator helpers
// ---------------------------------------------------------------------------

/// Anchor instruction discriminator: SHA-256(`"global:callback"`)[..8].
pub fn callback_discriminator() -> [u8; 8] {
    hash(b"global:callback").to_bytes()[..8]
        .try_into()
        .expect("8 bytes from 32-byte hash")
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
/// 2. `prover`           — mutable, signer (coordinator keypair)
/// 3. `callback_program` — not mutable, not signer
pub fn build_callback_instruction(
    program_id: Pubkey,
    request_id: &[u8; 32],
    prover_pubkey: Pubkey,
    callback_program: Pubkey,
    proof: &[u8],
    public_inputs: &[Vec<u8>],
    result: &[u8],
) -> (Instruction, Pubkey, Pubkey) {
    let (request_metadata_pda, _) =
        Pubkey::find_program_address(&[b"request", request_id.as_ref()], &program_id);
    let (result_account_pda, _) =
        Pubkey::find_program_address(&[b"result", request_id.as_ref()], &program_id);

    let accounts = vec![
        AccountMeta::new(request_metadata_pda, false),
        AccountMeta::new(result_account_pda, false),
        AccountMeta::new(prover_pubkey, true),
        AccountMeta::new_readonly(callback_program, false),
    ];

    let data = build_callback_instruction_data(proof, public_inputs, result);

    let ix = Instruction {
        program_id,
        accounts,
        data,
    };

    (ix, request_metadata_pda, result_account_pda)
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

    let callback_program = Pubkey::new_from_array(meta.callback_program);

    // Build instruction (public_inputs placeholder for Phase 5.1).
    let (ix, _, _) = build_callback_instruction(
        *program_id,
        &response.request_id,
        keypair.pubkey(),
        callback_program,
        &response.proof,
        &[], // TODO Phase 6.1: real public inputs
        &response.result,
    );

    // Send with retries.
    for attempt in 0..=max_retries {
        let blockhash = rpc
            .get_latest_blockhash()
            .await
            .context("get_latest_blockhash")?;

        let tx = Transaction::new_signed_with_payer(
            &[ix.clone()],
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
    fn instruction_has_four_accounts() {
        let program_id = Pubkey::new_unique();
        let request_id = [0u8; 32];
        let prover = Pubkey::new_unique();
        let cb_prog = Pubkey::new_unique();

        let (ix, _, _) =
            build_callback_instruction(program_id, &request_id, prover, cb_prog, &[], &[], &[]);

        assert_eq!(ix.accounts.len(), 4);
        // Account 2 (prover) must be a signer.
        assert!(ix.accounts[2].is_signer);
        // Accounts 0 and 1 must be mutable but not signers.
        assert!(ix.accounts[0].is_writable && !ix.accounts[0].is_signer);
        assert!(ix.accounts[1].is_writable && !ix.accounts[1].is_signer);
        // Account 3 (callback_program) must be read-only.
        assert!(!ix.accounts[3].is_writable && !ix.accounts[3].is_signer);
    }

    #[test]
    fn pdas_are_derived_from_request_id() {
        let program_id = Pubkey::new_unique();
        let request_id = [7u8; 32];
        let prover = Pubkey::new_unique();
        let cb_prog = Pubkey::new_unique();

        let (_, req_pda, res_pda) =
            build_callback_instruction(program_id, &request_id, prover, cb_prog, &[], &[], &[]);

        let (expected_req, _) =
            Pubkey::find_program_address(&[b"request", &request_id], &program_id);
        let (expected_res, _) =
            Pubkey::find_program_address(&[b"result", &request_id], &program_id);

        assert_eq!(req_pda, expected_req);
        assert_eq!(res_pda, expected_res);
    }
}
