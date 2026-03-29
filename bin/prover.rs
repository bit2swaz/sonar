use std::env;

use anyhow::Context;
use sonar_common::{config::Config, tracing_init::init_tracing};
use sonar_prover::service::run_redis_service;
use tokio::sync::watch;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path =
        env::var("SONAR_CONFIG").unwrap_or_else(|_| "config/default.toml".to_string());
    let config = Config::load(&config_path)
        .with_context(|| format!("failed to load prover config from {config_path}"))?;

    init_tracing(&config.observability.log_level);
    configure_prover_environment(&config);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_task = tokio::spawn(async move {
        if let Err(error) = wait_for_shutdown_signal().await {
            tracing::error!(%error, "failed while waiting for shutdown signal");
        }
        let _ = shutdown_tx.send(true);
    });

    info!(config_path, "starting sonar prover service");
    let result = run_redis_service(&config, shutdown_rx).await;
    shutdown_task.abort();
    result
}

fn configure_prover_environment(config: &Config) {
    if env::var("SP1_PROVER").is_ok() {
        return;
    }

    let prover = if config.prover.mock_prover {
        "mock"
    } else {
        "cpu"
    };
    env::set_var("SP1_PROVER", prover);
}

async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut terminate =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("failed to listen for Ctrl-C")?;
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to listen for Ctrl-C")?;
    }

    info!("shutdown signal received");
    Ok(())
}
