//! Sonar coordinator binary.
//!
//! Spawns two tasks:
//! - **Listener**: subscribes to Solana program logs, detects `request`
//!   events, pushes [`ProverJob`]s to `sonar:jobs`.
//! - **Callback worker**: pops [`ProverResponse`]s from `sonar:responses` and
//!   submits `callback` transactions on-chain.
//!
//! Configuration is loaded from the file at `$SONAR_CONFIG_PATH`
//! (defaults to `config/default.toml`).  The coordinator's signing keypair
//! must be supplied via `$SONAR_COORDINATOR_KEYPAIR_PATH` (a JSON file
//! containing a `[u8; 64]` byte array in Solana CLI format), or a fresh
//! ephemeral keypair is generated (development only).

use std::sync::Arc;

use anyhow::Context as _;
use solana_sdk::signature::Keypair;
use sonar_common::config::Config;
use sonar_coordinator::{
    callback::{run_callback_worker, CallbackConfig},
    dispatcher,
    listener::{run_listener, ListenerConfig, PROGRAM_ID_STR},
};
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Tracing ───────────────────────────────────────────────────────────
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // ── Config ────────────────────────────────────────────────────────────
    let config_path =
        std::env::var("SONAR_CONFIG_PATH").unwrap_or_else(|_| "config/default.toml".to_string());
    let cfg =
        Config::load(&config_path).with_context(|| format!("load config from {config_path}"))?;

    info!("Config loaded from {config_path}");

    // ── Keypair ───────────────────────────────────────────────────────────
    let keypair: Arc<Keypair> = match std::env::var("SONAR_COORDINATOR_KEYPAIR_PATH") {
        Ok(path) => {
            let json = std::fs::read_to_string(&path)
                .with_context(|| format!("read keypair file: {path}"))?;
            let bytes: Vec<u8> = serde_json::from_str(&json).context("deserialise keypair JSON")?;
            Arc::new(Keypair::try_from(bytes.as_slice()).context("construct Keypair from bytes")?)
        },
        Err(_) => {
            let kp = Keypair::new();
            info!("No SONAR_COORDINATOR_KEYPAIR_PATH — using ephemeral keypair (dev only)");
            Arc::new(kp)
        },
    };

    // ── Shutdown channel ──────────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // ── Listener task ─────────────────────────────────────────────────────
    let listener_cfg = ListenerConfig {
        ws_url: cfg.network.ws_url.clone(),
        rpc_url: cfg.network.rpc_url.clone(),
        redis_url: cfg.coordinator.redis_url.clone(),
        jobs_queue: dispatcher::JOBS_QUEUE.to_string(),
    };
    let listener_rx = shutdown_rx.clone();
    let listener_handle =
        tokio::spawn(async move { run_listener(listener_cfg, listener_rx).await });

    // ── Callback worker task ──────────────────────────────────────────────
    let callback_cfg = CallbackConfig {
        redis_url: cfg.coordinator.redis_url.clone(),
        responses_queue: dispatcher::RESPONSES_QUEUE.to_string(),
        rpc_url: cfg.network.rpc_url.clone(),
        program_id_str: PROGRAM_ID_STR.to_string(),
        prover_keypair: Arc::clone(&keypair),
        blpop_timeout_secs: 2.0,
        max_retries: 3,
    };
    let callback_rx = shutdown_rx.clone();
    let callback_handle =
        tokio::spawn(async move { run_callback_worker(callback_cfg, callback_rx).await });

    info!("Coordinator running — press Ctrl+C to stop");

    // ── Wait for SIGINT / SIGTERM ─────────────────────────────────────────
    tokio::signal::ctrl_c()
        .await
        .context("await ctrl_c signal")?;

    info!("Shutdown signal received — stopping tasks");
    let _ = shutdown_tx.send(true);

    // ── Join tasks ────────────────────────────────────────────────────────
    if let Err(e) = listener_handle.await {
        error!("Listener task panicked: {e:?}");
    }
    if let Err(e) = callback_handle.await {
        error!("Callback task panicked: {e:?}");
    }

    info!("Coordinator stopped");
    Ok(())
}
