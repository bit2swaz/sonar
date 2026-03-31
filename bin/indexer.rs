use anyhow::Context as _;
use sonar_common::config::Config;
use sonar_indexer::{
    db::{connect_pool, run_migrations, DatabaseConfig},
    server::start_server,
};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let config_path =
        std::env::var("SONAR_CONFIG").unwrap_or_else(|_| "config/default.toml".into());
    let config = Config::load(&config_path).context("failed to load config")?;

    let pool = connect_pool(&DatabaseConfig {
        database_url: config.indexer.database_url.clone(),
        max_connections: 10,
    })
    .await
    .context("failed to connect to database")?;

    run_migrations(&pool)
        .await
        .context("failed to run migrations")?;

    info!(
        "Starting indexer HTTP server on port {}",
        config.indexer.http_port
    );

    start_server(pool, config.indexer.http_port)
        .await
        .context("indexer HTTP server exited unexpectedly")
}
