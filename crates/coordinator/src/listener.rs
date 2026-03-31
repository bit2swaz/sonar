//! Listener — subscribes to Sonar program logs and dispatches [`ProverJob`]s.
//!
//! When the on-chain `request` instruction succeeds the program emits
//! `"sonar:request:<64-char hex request_id>"` via `msg!`.  This module:
//!
//! 1. Subscribes to Solana WebSocket logs mentioning the Sonar program.
//! 2. Detects the `sonar:request:` log pattern.
//! 3. Derives the `RequestMetadata` PDA and fetches its account data.
//! 4. Decodes the account, builds a [`ProverJob`], and pushes it to Redis.
//!
//! **Phase 5.1 note:** `ProverJob.inputs` is set to `Vec::new()`.  Phase 6.1
//! will enrich jobs with real inputs fetched from the indexer.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use futures_util::StreamExt as _;
use solana_client::{
    nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient},
    rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};
use tracing::{debug, error, info, warn};

use sonar_common::types::{ProverJob, Pubkey as CommonPubkey};

use crate::dispatcher;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Sonar on-chain program ID.
pub const PROGRAM_ID_STR: &str = "EE2sQ2VRa1hY3qjPQ1PEwuPZX6dGwTZwHMCumWrGn3sV";

/// Prefix emitted by `msg!("sonar:request:{}", hex_id)` in the program.
const LOG_REQUEST_PREFIX: &str = "Program log: sonar:request:";

// ---------------------------------------------------------------------------
// On-chain account layout (manual borsh decode — no extra dep)
// ---------------------------------------------------------------------------

/// Decoded mirror of the on-chain `RequestMetadata` Anchor account.
#[derive(Debug, Clone)]
pub struct OnChainRequestMetadata {
    pub request_id: [u8; 32],
    pub payer: [u8; 32],
    pub callback_program: [u8; 32],
    pub result_account: [u8; 32],
    pub computation_id: [u8; 32],
    pub deadline: u64,
    pub fee: u64,
    /// 0 = Pending, 1 = Completed, 2 = Refunded
    pub status: u8,
    pub completed_at: Option<u64>,
    pub bump: u8,
}

// ---------------------------------------------------------------------------
// Pure parsing helpers — all fully unit-testable
// ---------------------------------------------------------------------------

/// Scan `logs` for a `"Program log: sonar:request:<hex>"` entry and return
/// the decoded 32-byte request ID, or `None` if none is found.
pub fn parse_request_id_from_logs(logs: &[String]) -> Option<[u8; 32]> {
    for line in logs {
        if let Some(hex) = line.strip_prefix(LOG_REQUEST_PREFIX) {
            return parse_hex32(hex.trim());
        }
    }
    None
}

/// Decode a 64-character lower-hex string into 32 bytes.
pub fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Decode an Anchor `RequestMetadata` account buffer.
///
/// Layout (after the mandatory 8-byte Anchor discriminator):
/// ```text
/// [u8; 32]       request_id
/// [u8; 32]       payer
/// [u8; 32]       callback_program
/// [u8; 32]       result_account
/// [u8; 32]       computation_id
/// u64 LE         deadline
/// u64 LE         fee
/// u8             status  (0=Pending, 1=Completed, 2=Refunded)
/// u8             completed_at tag (0=None, 1=Some)
/// u64 LE         completed_at value (only present when tag=1)
/// u8             bump
/// ```
#[allow(unused_assignments)] // `c` is incremented by the take! macro even after the last field
pub fn decode_request_metadata(data: &[u8]) -> Option<OnChainRequestMetadata> {
    // Minimum byte count (tag=0 path): 8 + 32*5 + 8 + 8 + 1 + 1 + 1 = 179
    if data.len() < 179 {
        return None;
    }
    // Skip 8-byte Anchor discriminator.
    let d = &data[8..];
    let mut c = 0usize;

    macro_rules! take {
        ($n:expr) => {{
            if c + $n > d.len() {
                return None;
            }
            let s = &d[c..c + $n];
            c += $n;
            s
        }};
    }

    let request_id: [u8; 32] = take!(32).try_into().ok()?;
    let payer: [u8; 32] = take!(32).try_into().ok()?;
    let callback_program: [u8; 32] = take!(32).try_into().ok()?;
    let result_account: [u8; 32] = take!(32).try_into().ok()?;
    let computation_id: [u8; 32] = take!(32).try_into().ok()?;
    let deadline = u64::from_le_bytes(take!(8).try_into().ok()?);
    let fee = u64::from_le_bytes(take!(8).try_into().ok()?);
    let status = take!(1)[0];
    let tag = take!(1)[0];
    let completed_at = if tag == 1 {
        let v = u64::from_le_bytes(take!(8).try_into().ok()?);
        Some(v)
    } else {
        None
    };
    let bump = take!(1)[0];

    Some(OnChainRequestMetadata {
        request_id,
        payer,
        callback_program,
        result_account,
        computation_id,
        deadline,
        fee,
        status,
        completed_at,
        bump,
    })
}

// ---------------------------------------------------------------------------
// Job building
// ---------------------------------------------------------------------------

/// Build a [`ProverJob`] from a decoded `RequestMetadata`.
///
/// `inputs` is left empty for Phase 5.1; Phase 6.1 will populate it from the
/// indexer before dispatching.
pub fn build_prover_job(meta: &OnChainRequestMetadata) -> ProverJob {
    ProverJob {
        request_id: meta.request_id,
        computation_id: meta.computation_id,
        inputs: Vec::new(), // TODO Phase 6.1: fetch from indexer
        deadline: meta.deadline,
        fee: meta.fee,
        callback_program: CommonPubkey::new(meta.callback_program),
        result_account: CommonPubkey::new(meta.result_account),
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Runtime configuration for the listener task.
pub struct ListenerConfig {
    pub ws_url: String,
    pub rpc_url: String,
    pub redis_url: String,
    pub jobs_queue: String,
}

// ---------------------------------------------------------------------------
// Main listener task
// ---------------------------------------------------------------------------

/// Subscribe to Sonar program logs and forward new `request` events to the
/// Redis jobs queue.  Returns when `shutdown` receives `true`.
pub async fn run_listener(
    config: ListenerConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let program_id = Pubkey::from_str(PROGRAM_ID_STR).expect("valid program ID");

    let rpc = RpcClient::new_with_commitment(config.rpc_url.clone(), CommitmentConfig::confirmed());

    let redis_client =
        redis::Client::open(config.redis_url.as_str()).context("redis client open")?;
    let mut redis_conn = redis_client
        .get_multiplexed_async_connection()
        .await
        .context("redis connect")?;

    // Track dispatched request IDs to avoid double-processing.
    let seen: Arc<Mutex<HashSet<[u8; 32]>>> = Arc::new(Mutex::new(HashSet::new()));

    let pubsub = PubsubClient::new(&config.ws_url)
        .await
        .context("pubsub connect")?;

    let filter = RpcTransactionLogsFilter::Mentions(vec![PROGRAM_ID_STR.to_string()]);
    let log_cfg = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::confirmed()),
    };

    let (mut stream, _unsubscribe) = pubsub
        .logs_subscribe(filter, log_cfg)
        .await
        .context("logs_subscribe")?;

    info!("Listener started — watching {PROGRAM_ID_STR}");

    loop {
        tokio::select! {
            maybe = stream.next() => {
                match maybe {
                    Some(notification) => {
                        let logs_resp = notification.value;
                        if let Err(e) = handle_log_event(
                            &rpc,
                            &mut redis_conn,
                            &config.jobs_queue,
                            &program_id,
                            &seen,
                            logs_resp,
                        )
                        .await
                        {
                            warn!("handle_log_event: {e:#}");
                        }
                    }
                    None => {
                        error!("log subscription stream closed");
                        anyhow::bail!("stream closed");
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("Listener shutting down");
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn handle_log_event(
    rpc: &RpcClient,
    redis_conn: &mut redis::aio::MultiplexedConnection,
    jobs_queue: &str,
    program_id: &Pubkey,
    seen: &Arc<Mutex<HashSet<[u8; 32]>>>,
    logs_resp: solana_client::rpc_response::RpcLogsResponse,
) -> anyhow::Result<()> {
    // Skip failed transactions.
    if logs_resp.err.is_some() {
        return Ok(());
    }

    let Some(request_id) = parse_request_id_from_logs(&logs_resp.logs) else {
        return Ok(()); // Not a request event
    };

    // Deduplicate: skip if already dispatched.
    {
        let mut guard = seen.lock().unwrap();
        if !guard.insert(request_id) {
            debug!("Duplicate request — skipping");
            return Ok(());
        }
    }

    debug!("Request detected: sig={}", &logs_resp.signature);

    // Derive and fetch the RequestMetadata PDA.
    let (pda, _bump) = Pubkey::find_program_address(&[b"request", &request_id], program_id);

    let account_data = rpc
        .get_account_data(&pda)
        .await
        .context("get_account_data for RequestMetadata")?;

    let meta = decode_request_metadata(&account_data).context("decode RequestMetadata account")?;

    // Only dispatch Pending requests.
    if meta.status != 0 {
        debug!("Request not pending — skipping");
        return Ok(());
    }

    let job = build_prover_job(&meta);
    dispatcher::push_job(redis_conn, jobs_queue, &job).await?;

    info!("Dispatched job for request {}", hex_encode(&request_id));
    Ok(())
}

/// Encode a byte slice as lower-case hex (no external crate).
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_request_id_from_logs ---

    #[test]
    fn detects_request_log() {
        let id = [0xABu8; 32];
        let hex: String = id.iter().map(|b| format!("{:02x}", b)).collect();
        let logs = vec![
            "Program EE2s... invoke [1]".to_string(),
            format!("Program log: sonar:request:{}", hex),
            "Program EE2s... success".to_string(),
        ];
        let parsed = parse_request_id_from_logs(&logs).expect("should detect");
        assert_eq!(parsed, id);
    }

    #[test]
    fn returns_none_when_no_request_log() {
        let logs = vec![
            "Program EE2s... invoke [1]".to_string(),
            "Program log: Instruction: Request".to_string(),
            "Program EE2s... success".to_string(),
        ];
        assert!(parse_request_id_from_logs(&logs).is_none());
    }

    #[test]
    fn returns_none_on_empty_logs() {
        assert!(parse_request_id_from_logs(&[]).is_none());
    }

    // --- parse_hex32 ---

    #[test]
    fn parse_hex32_all_zeros() {
        let s = "0".repeat(64);
        assert_eq!(parse_hex32(&s), Some([0u8; 32]));
    }

    #[test]
    fn parse_hex32_all_ff() {
        let s = "ff".repeat(32);
        assert_eq!(parse_hex32(&s), Some([0xFFu8; 32]));
    }

    #[test]
    fn parse_hex32_rejects_short() {
        assert!(parse_hex32("deadbeef").is_none());
    }

    #[test]
    fn parse_hex32_rejects_invalid_char() {
        let s = "g".repeat(64); // 'g' is not a valid hex digit
        assert!(parse_hex32(&s).is_none());
    }

    #[test]
    fn parse_hex32_roundtrip() {
        let original = [0x12u8; 32];
        let hex: String = original.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(parse_hex32(&hex), Some(original));
    }

    // --- decode_request_metadata ---

    fn make_metadata_bytes() -> Vec<u8> {
        // 8-byte discriminator (arbitrary for testing)
        let mut data = vec![0u8; 8];
        // request_id
        data.extend_from_slice(&[1u8; 32]);
        // payer
        data.extend_from_slice(&[2u8; 32]);
        // callback_program
        data.extend_from_slice(&[3u8; 32]);
        // result_account
        data.extend_from_slice(&[4u8; 32]);
        // computation_id
        data.extend_from_slice(&[5u8; 32]);
        // deadline: 9999 as u64 LE
        data.extend_from_slice(&9999u64.to_le_bytes());
        // fee: 123 as u64 LE
        data.extend_from_slice(&123u64.to_le_bytes());
        // status: 0 (Pending)
        data.push(0);
        // completed_at: None (tag = 0)
        data.push(0);
        // bump: 255
        data.push(255);
        data
    }

    #[test]
    fn decode_metadata_happy_path() {
        let data = make_metadata_bytes();
        let meta = decode_request_metadata(&data).expect("should decode");
        assert_eq!(meta.request_id, [1u8; 32]);
        assert_eq!(meta.callback_program, [3u8; 32]);
        assert_eq!(meta.result_account, [4u8; 32]);
        assert_eq!(meta.computation_id, [5u8; 32]);
        assert_eq!(meta.deadline, 9999);
        assert_eq!(meta.fee, 123);
        assert_eq!(meta.status, 0);
        assert!(meta.completed_at.is_none());
        assert_eq!(meta.bump, 255);
    }

    #[test]
    fn decode_metadata_with_completed_at() {
        let mut data = make_metadata_bytes();
        // Patch: change completed_at tag to 1 and append the value.
        // completed_at tag is at offset 8 + 32*5 + 8 + 8 + 1 = 185
        data[185] = 1;
        // Insert 8 bytes for the slot value (77) after tag.
        let tail = data.split_off(186);
        data.extend_from_slice(&77u64.to_le_bytes());
        data.extend(tail);
        let meta = decode_request_metadata(&data).expect("should decode");
        assert_eq!(meta.completed_at, Some(77));
    }

    #[test]
    fn decode_metadata_rejects_short_buffer() {
        assert!(decode_request_metadata(&[0u8; 10]).is_none());
    }

    // --- build_prover_job ---

    #[test]
    fn build_job_from_metadata() {
        let data = make_metadata_bytes();
        let meta = decode_request_metadata(&data).unwrap();
        let job = build_prover_job(&meta);
        assert_eq!(job.request_id, [1u8; 32]);
        assert_eq!(job.computation_id, [5u8; 32]);
        assert_eq!(job.deadline, 9999);
        assert_eq!(job.fee, 123);
        assert_eq!(*job.callback_program.as_bytes(), [3u8; 32]);
        assert_eq!(*job.result_account.as_bytes(), [4u8; 32]);
        assert!(job.inputs.is_empty(), "Phase 5.1: inputs are empty");
    }
}
