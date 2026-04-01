//! Listener — subscribes to Sonar program logs and dispatches [`ProverJob`]s.
//!
//! When the on-chain `request` instruction succeeds the program emits
//! `"sonar:request:<64-char hex request_id>"` and
//! `"sonar:inputs:<hex-encoded inputs>"` via `msg!`.  This module:
//!
//! 1. Subscribes to Solana WebSocket logs mentioning the Sonar program.
//! 2. Detects the `sonar:request:` log pattern.
//! 3. Derives the `RequestMetadata` PDA and fetches its account data.
//! 4. Decodes the account, builds a [`ProverJob`], and pushes it to Redis.
//!
//! For **HistoricalAvg** requests the coordinator:
//! - Parses the `sonar:inputs:` log to extract (pubkey, from_slot, to_slot).
//! - Fetches lamport balances from the indexer HTTP API.
//! - Encodes the balance list as `bincode::serialize(&Vec<u64>)` and stores it
//!   in `ProverJob.inputs`.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

/// Prefix emitted by `msg!("sonar:inputs:{}", hex_inputs)` in the program.
const LOG_INPUTS_PREFIX: &str = "Program log: sonar:inputs:";

/// Byte length of historical-average inputs: pubkey[32] + from_slot[8] + to_slot[8].
pub const HISTORICAL_AVG_INPUTS_LEN: usize = 48;

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

/// Decoded on-chain inputs for a HistoricalAvg request.
#[derive(Debug, Clone)]
pub struct HistoricalAvgInputs {
    pub pubkey: [u8; 32],
    pub from_slot: u64,
    pub to_slot: u64,
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

/// Scan `logs` for a `"Program log: sonar:inputs:<hex>"` entry and return
/// the decoded bytes, or `None` if not present.
pub fn parse_inputs_from_logs(logs: &[String]) -> Option<Vec<u8>> {
    for line in logs {
        if let Some(hex) = line.strip_prefix(LOG_INPUTS_PREFIX) {
            let hex = hex.trim();
            if hex.len() % 2 != 0 {
                return None;
            }
            let mut out = Vec::with_capacity(hex.len() / 2);
            for chunk in hex.as_bytes().chunks_exact(2) {
                let hi = hex_nibble(chunk[0])?;
                let lo = hex_nibble(chunk[1])?;
                out.push((hi << 4) | lo);
            }
            return Some(out);
        }
    }
    None
}

/// Decode 48 raw input bytes into `(pubkey[32], from_slot, to_slot)`.
pub fn decode_historical_avg_inputs(raw: &[u8]) -> Option<HistoricalAvgInputs> {
    if raw.len() != HISTORICAL_AVG_INPUTS_LEN {
        return None;
    }
    let pubkey: [u8; 32] = raw[..32].try_into().ok()?;
    let from_slot = u64::from_le_bytes(raw[32..40].try_into().ok()?);
    let to_slot = u64::from_le_bytes(raw[40..48].try_into().ok()?);
    Some(HistoricalAvgInputs {
        pubkey,
        from_slot,
        to_slot,
    })
}

/// Encode HistoricalAvg request inputs as 48 raw bytes.
pub fn encode_historical_avg_inputs(pubkey: &[u8; 32], from_slot: u64, to_slot: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HISTORICAL_AVG_INPUTS_LEN);
    out.extend_from_slice(pubkey);
    out.extend_from_slice(&from_slot.to_le_bytes());
    out.extend_from_slice(&to_slot.to_le_bytes());
    out
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

/// Build a [`ProverJob`] from a decoded `RequestMetadata` and pre-fetched
/// prover inputs.
pub fn build_prover_job(meta: &OnChainRequestMetadata, inputs: Vec<u8>) -> ProverJob {
    ProverJob {
        request_id: meta.request_id,
        computation_id: meta.computation_id,
        inputs,
        deadline: meta.deadline,
        fee: meta.fee,
        callback_program: CommonPubkey::new(meta.callback_program),
        result_account: CommonPubkey::new(meta.result_account),
    }
}

// ---------------------------------------------------------------------------
// Indexer client helper
// ---------------------------------------------------------------------------

/// Fetch the ordered lamport-balance history for `pubkey` in `[from, to]`
/// from the indexer HTTP API and return a bincode-serialised `Vec<u64>`.
pub async fn fetch_historical_avg_inputs(
    indexer_url: &str,
    inputs: &HistoricalAvgInputs,
) -> anyhow::Result<Vec<u8>> {
    let pubkey_b58 = bs58::encode(&inputs.pubkey).into_string();
    let url = format!(
        "{indexer_url}/account_history/{pubkey_b58}?from_slot={}&to_slot={}",
        inputs.from_slot, inputs.to_slot,
    );

    let balances: Vec<u64> = reqwest::get(&url)
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("indexer returned error for {url}"))?
        .json()
        .await
        .with_context(|| format!("failed to parse indexer response from {url}"))?;

    bincode::serialize(&balances).context("failed to bincode-encode balances")
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
    /// Base URL of the indexer HTTP server, e.g. `http://localhost:8080`.
    pub indexer_url: String,
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

    // Use the `All` filter instead of `Mentions` so we don't miss events.
    // The Agave 3.x validator / SDK 2.x client combination can silently drop
    // `Mentions`-filtered notifications; processing every event and filtering
    // in-process is more reliable.  `handle_log_event` exits early for any
    // transaction that does not contain a `sonar:request:` log line.
    let filter = RpcTransactionLogsFilter::All;
    let log_cfg = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::confirmed()),
    };

    let (mut stream, _unsubscribe) = pubsub
        .logs_subscribe(filter, log_cfg)
        .await
        .context("logs_subscribe")?;

    info!("Listener started — watching {PROGRAM_ID_STR}");

    // Polling fallback: every 2 s we call `getSignaturesForAddress` +
    // Polling fallback: every 2 s we call `getProgramAccounts` for the Sonar
    // program to discover any pending `RequestMetadata` accounts.  This
    // compensates for the Agave 3.x / SDK 2.x WebSocket subscription
    // compatibility issue where `logsSubscribe` silently delivers no events.
    let mut polling_ticker = tokio::time::interval(Duration::from_secs(2));
    polling_ticker.tick().await; // consume the initial tick (fires immediately)
    let mut ws_stream_open = true;

    loop {
        tokio::select! {
            maybe = async { if ws_stream_open { stream.next().await } else { std::future::pending().await } } => {
                match maybe {
                    Some(notification) => {
                        let logs_resp = notification.value;
                        if let Err(e) = handle_log_event(
                            &rpc,
                            &mut redis_conn,
                            &config.jobs_queue,
                            &config.indexer_url,
                            &program_id,
                            &seen,
                            logs_resp,
                        )
                        .await
                        {
                            warn!("handle_log_event (ws): {e:#}");
                        }
                    }
                    None => {
                        // Stream closed — not fatal, rely on polling as fallback.
                        // Mark ws_stream_open=false so we don't keep polling a
                        // closed stream (which would immediately return None and
                        // starve the ticker branch).
                        error!("log subscription stream closed (will rely on polling)");
                        ws_stream_open = false;
                    }
                }
            }
            _ = polling_ticker.tick() => {
                if let Err(e) = poll_pending_requests(
                    &config.rpc_url,
                    &program_id,
                    &mut redis_conn,
                    &config.jobs_queue,
                    &config.indexer_url,
                    &seen,
                )
                .await
                {
                    warn!("poll_pending_requests: {e:#}");
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

/// Poll for pending `RequestMetadata` accounts owned by the Sonar program and
/// dispatch any that have not yet been processed to the Redis jobs queue.
///
/// Uses `getProgramAccounts` with a discriminator filter so it works reliably
/// even when `getSignaturesForAddress` does not index program invocations
/// (which is the case on `solana-test-validator` 3.0.13).
async fn poll_pending_requests(
    rpc_url: &str,
    program_id: &Pubkey,
    redis_conn: &mut redis::aio::MultiplexedConnection,
    jobs_queue: &str,
    indexer_url: &str,
    seen: &Arc<Mutex<HashSet<[u8; 32]>>>,
) -> anyhow::Result<()> {
    // Anchor discriminator for `RequestMetadata` = sha256("account:RequestMetadata")[0..8]
    // = [14, 83, 46, 148, 18, 10, 201, 25]
    const DISC: [u8; 8] = [14, 83, 46, 148, 18, 10, 201, 25];
    let disc_b64 = base64_encode(&DISC);

    // status=Pending is 0 at byte offset 184
    // Layout: 8 disc + 32*5 pubkeys + 8 deadline + 8 fee = 184
    let status_b64 = base64_encode(&[0u8]);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getProgramAccounts",
        "params": [
            program_id.to_string(),
            {
                "encoding": "base64",
                "commitment": "confirmed",
                "filters": [
                    {"memcmp": {"offset": 0, "bytes": disc_b64, "encoding": "base64"}},
                    {"memcmp": {"offset": 184, "bytes": status_b64, "encoding": "base64"}}
                ]
            }
        ]
    });

    let resp: serde_json::Value = reqwest::Client::new()
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .context("getProgramAccounts HTTP")?
        .json()
        .await
        .context("getProgramAccounts JSON parse")?;

    let accounts = match resp["result"].as_array() {
        Some(a) => a.clone(),
        None => {
            if resp["error"].is_object() {
                warn!("polling: getProgramAccounts error: {}", resp["error"]);
            }
            debug!("polling: no pending RequestMetadata accounts for {program_id}");
            return Ok(());
        },
    };

    if accounts.is_empty() {
        debug!("polling: no pending RequestMetadata accounts for {program_id}");
        return Ok(());
    }

    info!(
        "polling: found {} pending RequestMetadata account(s)",
        accounts.len()
    );

    for entry in &accounts {
        let pda_str = entry["pubkey"].as_str().unwrap_or("");
        let data_arr = entry["account"]["data"].as_array();

        let raw_bytes = if let Some(arr) = data_arr {
            if let Some(b64) = arr.first().and_then(|v| v.as_str()) {
                base64_decode(b64).unwrap_or_default()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let meta = match decode_request_metadata(&raw_bytes) {
            Some(m) => m,
            None => {
                warn!("polling: failed to decode RequestMetadata at {pda_str}");
                continue;
            },
        };

        // Deduplicate: skip if already dispatched.
        {
            let mut guard = seen.lock().unwrap();
            if !guard.insert(meta.request_id) {
                debug!(
                    "polling: duplicate request {} — skipping",
                    hex_encode(&meta.request_id)
                );
                continue;
            }
        }

        info!(
            "polling: dispatching job for request {}",
            hex_encode(&meta.request_id)
        );

        if let Err(e) =
            process_request_metadata(rpc_url, redis_conn, jobs_queue, indexer_url, pda_str, &meta)
                .await
        {
            warn!("polling: process_request_metadata: {e:#}");
            // Remove from seen so we retry next poll.
            let mut guard = seen.lock().unwrap();
            guard.remove(&meta.request_id);
        }
    }

    Ok(())
}

/// Fetch the creation transaction for a `RequestMetadata` PDA and build a
/// [`ProverJob`] from its logs + on-chain metadata.
async fn process_request_metadata(
    rpc_url: &str,
    redis_conn: &mut redis::aio::MultiplexedConnection,
    jobs_queue: &str,
    indexer_url: &str,
    pda_str: &str,
    meta: &OnChainRequestMetadata,
) -> anyhow::Result<()> {
    // Get the most recent signature for this PDA (the creation tx).
    let sig_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSignaturesForAddress",
        "params": [
            pda_str,
            {"limit": 1, "commitment": "confirmed"}
        ]
    });

    let sig_resp: serde_json::Value = reqwest::Client::new()
        .post(rpc_url)
        .json(&sig_body)
        .send()
        .await
        .context("getSignaturesForAddress HTTP")?
        .json()
        .await
        .context("getSignaturesForAddress JSON")?;

    let sig_str = sig_resp["result"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|s| s["signature"].as_str())
        .ok_or_else(|| anyhow::anyhow!("no signatures for PDA {pda_str}"))?
        .to_string();

    // Fetch transaction logs.
    let logs = fetch_tx_logs(rpc_url, &sig_str).await?.unwrap_or_default();

    info!(
        "process_request_metadata: sig={sig_str} logs={} lines",
        logs.len()
    );
    for l in &logs {
        info!("  log: {l}");
    }

    // Parse raw inputs from logs and enrich for known computation types.
    let raw_inputs = parse_inputs_from_logs(&logs).unwrap_or_default();
    info!(
        "process_request_metadata: raw_inputs len={}",
        raw_inputs.len()
    );
    let prover_inputs = enrich_inputs(&meta.computation_id, &raw_inputs, indexer_url).await?;
    info!(
        "process_request_metadata: prover_inputs len={}",
        prover_inputs.len()
    );

    let job = build_prover_job(meta, prover_inputs);
    dispatcher::push_job(redis_conn, jobs_queue, &job).await?;

    info!(
        "Dispatched job for request {} (via polling)",
        hex_encode(&meta.request_id)
    );
    Ok(())
}

/// Fetch the `logMessages` for a confirmed transaction via a raw JSON-RPC call.
/// Returns `None` when the transaction is not yet available.
async fn fetch_tx_logs(rpc_url: &str, signature: &str) -> anyhow::Result<Option<Vec<String>>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "json",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0
            }
        ]
    });

    let response: serde_json::Value = reqwest::Client::new()
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .context("getTransaction HTTP")?
        .json::<serde_json::Value>()
        .await
        .context("getTransaction parse JSON")?;

    if response["result"].is_null() {
        return Ok(None);
    }

    let logs: Vec<String> = response["result"]["meta"]["logMessages"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(Some(logs))
}

/// Encode bytes as base64 (standard, no padding strip).
fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    // Simple base64 encoder — no external dep needed.
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let _ = write!(out, "{}", TABLE[(b0 >> 2) as usize] as char);
        let _ = write!(out, "{}", TABLE[((b0 & 3) << 4 | b1 >> 4) as usize] as char);
        if chunk.len() > 1 {
            let _ = write!(
                out,
                "{}",
                TABLE[((b1 & 0xf) << 2 | b2 >> 6) as usize] as char
            );
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            let _ = write!(out, "{}", TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode a base64 string to bytes. Returns `None` on invalid input.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    // Use solana_sdk which re-exports base64 indirectly, or hand-roll.
    let s = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            b'=' => Some(0),
            _ => None,
        }
    }
    for chunk in s.chunks(4) {
        if chunk.len() < 4 {
            return None;
        }
        let v = [
            val(chunk[0])?,
            val(chunk[1])?,
            val(chunk[2])?,
            val(chunk[3])?,
        ];
        out.push((v[0] << 2) | (v[1] >> 4));
        if chunk[2] != b'=' {
            out.push((v[1] << 4) | (v[2] >> 2));
        }
        if chunk[3] != b'=' {
            out.push((v[2] << 6) | v[3]);
        }
    }
    Some(out)
}

async fn handle_log_event(
    rpc: &RpcClient,
    redis_conn: &mut redis::aio::MultiplexedConnection,
    jobs_queue: &str,
    indexer_url: &str,
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

    // Parse raw inputs from logs and enrich for known computation types.
    let raw_inputs = parse_inputs_from_logs(&logs_resp.logs).unwrap_or_default();
    let prover_inputs = enrich_inputs(&meta.computation_id, &raw_inputs, indexer_url).await?;

    let job = build_prover_job(&meta, prover_inputs);
    dispatcher::push_job(redis_conn, jobs_queue, &job).await?;

    info!("Dispatched job for request {}", hex_encode(&request_id));
    Ok(())
}

/// Derive final prover `inputs` from the raw on-chain bytes.
///
/// For **HistoricalAvg** this means fetching lamport balances from the indexer
/// and returning them as a bincode-encoded `Vec<u64>`.
/// For all other computations the raw bytes are passed through unchanged.
async fn enrich_inputs(
    _computation_id: &[u8; 32],
    raw_inputs: &[u8],
    indexer_url: &str,
) -> anyhow::Result<Vec<u8>> {
    // Determine if this is a HistoricalAvg request by trying to decode the
    // 48-byte layout.  A concrete computation-ID comparison would require
    // loading the ELF here (heavy); the fixed input length acts as a
    // lightweight proxy while still being unambiguous in practice.
    if raw_inputs.len() == HISTORICAL_AVG_INPUTS_LEN {
        if let Some(ha_inputs) = decode_historical_avg_inputs(raw_inputs) {
            let encoded = fetch_historical_avg_inputs(indexer_url, &ha_inputs)
                .await
                .with_context(|| {
                    format!(
                        "failed to fetch historical_avg inputs for pubkey {}",
                        bs58::encode(&ha_inputs.pubkey).into_string()
                    )
                })?;
            return Ok(encoded);
        }
    }

    // Passthrough for fibonacci and any other fixed-format computations.
    Ok(raw_inputs.to_vec())
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

    // --- parse_inputs_from_logs ---

    #[test]
    fn detects_inputs_log() {
        let raw = vec![0x01u8, 0x02, 0xff];
        let hex: String = raw.iter().map(|b| format!("{:02x}", b)).collect();
        let logs = vec![
            format!("Program log: sonar:inputs:{}", hex),
            "Program EE2s... success".to_string(),
        ];
        assert_eq!(parse_inputs_from_logs(&logs), Some(raw));
    }

    #[test]
    fn returns_none_when_no_inputs_log() {
        let logs = vec!["Program EE2s... success".to_string()];
        assert!(parse_inputs_from_logs(&logs).is_none());
    }

    #[test]
    fn empty_inputs_hex_decodes_to_empty_vec() {
        let logs = vec!["Program log: sonar:inputs:".to_string()];
        assert_eq!(parse_inputs_from_logs(&logs), Some(Vec::new()));
    }

    // --- decode_historical_avg_inputs ---

    #[test]
    fn decode_historical_avg_inputs_roundtrip() {
        let pubkey = [0x42u8; 32];
        let from_slot = 1000u64;
        let to_slot = 2000u64;
        let raw = encode_historical_avg_inputs(&pubkey, from_slot, to_slot);
        let decoded = decode_historical_avg_inputs(&raw).expect("should decode");
        assert_eq!(decoded.pubkey, pubkey);
        assert_eq!(decoded.from_slot, from_slot);
        assert_eq!(decoded.to_slot, to_slot);
    }

    #[test]
    fn decode_historical_avg_inputs_rejects_wrong_len() {
        assert!(decode_historical_avg_inputs(&[0u8; 10]).is_none());
        assert!(decode_historical_avg_inputs(&[0u8; 49]).is_none());
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
    fn build_job_from_metadata_empty_inputs() {
        let data = make_metadata_bytes();
        let meta = decode_request_metadata(&data).unwrap();
        let job = build_prover_job(&meta, Vec::new());
        assert_eq!(job.request_id, [1u8; 32]);
        assert_eq!(job.computation_id, [5u8; 32]);
        assert_eq!(job.deadline, 9999);
        assert_eq!(job.fee, 123);
        assert_eq!(*job.callback_program.as_bytes(), [3u8; 32]);
        assert_eq!(*job.result_account.as_bytes(), [4u8; 32]);
        assert!(job.inputs.is_empty());
    }

    #[test]
    fn build_job_from_metadata_with_inputs() {
        let data = make_metadata_bytes();
        let meta = decode_request_metadata(&data).unwrap();
        let inputs = vec![1u8, 2, 3];
        let job = build_prover_job(&meta, inputs.clone());
        assert_eq!(job.inputs, inputs);
    }
}
