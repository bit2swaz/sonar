//! Config loading with `${ENV_VAR}` expansion.
//!
//! Call [`Config::load`] to read a TOML file from disk (all `${VAR}` patterns
//! are substituted with environment variables before parsing), or
//! [`Config::load_str`] to parse from a string directly (useful in tests).

use std::env;
use std::fs;

use anyhow::{anyhow, Context};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub network: NetworkConfig,
    pub strategy: StrategyConfig,
    pub rpc: RpcConfig,
    pub indexer: IndexerConfig,
    pub coordinator: CoordinatorConfig,
    pub prover: ProverConfig,
    pub observability: ObservabilityConfig,
}

// ---------------------------------------------------------------------------
// Sub-configs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    pub rpc_url: String,
    pub ws_url: String,
    pub chain_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    pub min_profit_floor_usd: f64,
    pub gas_buffer_multiplier: f64,
    pub max_gas_price_gwei: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcConfig {
    pub helius_api_key: String,
    pub helius_rpc_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexerConfig {
    pub geyser_plugin_path: String,
    pub database_url: String,
    pub concurrency: usize,
    /// Port for the indexer's HTTP query server.
    pub http_port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoordinatorConfig {
    pub redis_url: String,
    pub callback_timeout_seconds: u64,
    pub max_concurrent_jobs: usize,
    /// Base URL of the indexer HTTP server (e.g. `http://localhost:8080`).
    pub indexer_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProverConfig {
    pub sp1_proving_key_path: String,
    pub groth16_params_path: String,
    pub mock_prover: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservabilityConfig {
    pub log_level: String,
    pub metrics_port: u16,
}

// ---------------------------------------------------------------------------
// Config impl
// ---------------------------------------------------------------------------

impl Config {
    /// Load config from a TOML file on disk.  All `${VAR}` patterns are
    /// substituted with the corresponding environment variables before parsing.
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {path}"))?;
        let expanded = Self::expand_env_vars(&raw)?;
        Self::parse_toml(&expanded)
    }

    /// Parse a TOML string directly (after any desired env-var expansion).
    pub fn load_str(toml_str: &str) -> anyhow::Result<Self> {
        let expanded = Self::expand_env_vars(toml_str)?;
        Self::parse_toml(&expanded)
    }

    fn parse_toml(s: &str) -> anyhow::Result<Self> {
        toml::from_str(s).context("Failed to parse TOML config")
    }

    /// Replace every `${VAR_NAME}` occurrence with the value of the named
    /// environment variable.  Returns an error if any referenced variable is
    /// not set.
    fn expand_env_vars(input: &str) -> anyhow::Result<String> {
        let mut result = String::with_capacity(input.len());
        let mut remaining = input;

        while let Some(start) = remaining.find("${") {
            // Everything before `${` is literal.
            result.push_str(&remaining[..start]);
            remaining = &remaining[start + 2..];

            // Find the closing `}`.
            let end = remaining
                .find('}')
                .ok_or_else(|| anyhow!("Unclosed '${{' in config string"))?;

            let var_name = &remaining[..end];
            remaining = &remaining[end + 1..];

            let value = env::var(var_name).map_err(|_| anyhow!("Missing env var: {var_name}"))?;
            result.push_str(&value);
        }

        // Append whatever is left after the last `}`.
        result.push_str(remaining);
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// A fully-expanded TOML string that matches the Config struct exactly.
    fn valid_toml() -> &'static str {
        r#"
[network]
rpc_url  = "https://api.mainnet-beta.solana.com"
ws_url   = "wss://api.mainnet-beta.solana.com"
chain_id = "mainnet"

[strategy]
min_profit_floor_usd  = 0.10
gas_buffer_multiplier = 1.2
max_gas_price_gwei    = 2.0

[rpc]
helius_api_key = "test-key-123"
helius_rpc_url = "https://mainnet.helius-rpc.com"

[indexer]
geyser_plugin_path = "/opt/sonar/libsonar_indexer.so"
database_url       = "postgresql://postgres:password@localhost:5432/sonar"
concurrency        = 4
http_port          = 8080

[coordinator]
redis_url                = "redis://localhost:6379"
callback_timeout_seconds = 30
max_concurrent_jobs      = 8
indexer_url              = "http://localhost:8080"

[prover]
sp1_proving_key_path = "/opt/sonar/sp1.key"
groth16_params_path  = "/opt/sonar/groth16.params"
mock_prover          = false

[observability]
log_level    = "info"
metrics_port = 9090
"#
    }

    #[test]
    fn test_load_str_valid_config() {
        let cfg = Config::load_str(valid_toml()).unwrap();
        assert_eq!(cfg.network.chain_id, "mainnet");
        assert_eq!(cfg.network.rpc_url, "https://api.mainnet-beta.solana.com");
        assert!((cfg.strategy.min_profit_floor_usd - 0.10).abs() < f64::EPSILON);
        assert_eq!(cfg.rpc.helius_api_key, "test-key-123");
        assert_eq!(cfg.indexer.concurrency, 4);
        assert_eq!(cfg.coordinator.callback_timeout_seconds, 30);
        assert!(!cfg.prover.mock_prover);
        assert_eq!(cfg.observability.metrics_port, 9090);
    }

    #[test]
    fn test_expand_env_vars_substitutes_correctly() {
        env::set_var("TEST_VAR_SONAR_A", "hello");
        let out = Config::expand_env_vars("${TEST_VAR_SONAR_A}").unwrap();
        assert_eq!(out, "hello");
        env::remove_var("TEST_VAR_SONAR_A");
    }

    #[test]
    fn test_expand_env_vars_multiple() {
        env::set_var("TEST_VAR_SONAR_B", "foo");
        env::set_var("TEST_VAR_SONAR_C", "bar");
        let out = Config::expand_env_vars("${TEST_VAR_SONAR_B} and ${TEST_VAR_SONAR_C}").unwrap();
        assert_eq!(out, "foo and bar");
        env::remove_var("TEST_VAR_SONAR_B");
        env::remove_var("TEST_VAR_SONAR_C");
    }

    #[test]
    fn test_expand_env_vars_missing_var() {
        // Make sure the var is unset.
        env::remove_var("TEST_VAR_SONAR_MISSING_XYZ");
        let err = Config::expand_env_vars("${TEST_VAR_SONAR_MISSING_XYZ}").unwrap_err();
        assert!(
            err.to_string().contains("TEST_VAR_SONAR_MISSING_XYZ"),
            "error should name the missing var, got: {err}"
        );
    }

    #[test]
    fn test_expand_env_vars_no_vars() {
        let input = "just a plain string with no placeholders";
        let out = Config::expand_env_vars(input).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn test_load_str_invalid_toml() {
        let err = Config::load_str("this is { not valid toml !!!").unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("toml") || err.to_string().contains("parse"),
            "expected a TOML parse error, got: {err}"
        );
    }

    #[test]
    fn test_default_toml_loads_with_env() {
        // Set every env var referenced in config/default.toml.
        env::set_var("SOLANA_RPC_URL", "https://api.mainnet-beta.solana.com");
        env::set_var("SOLANA_WS_URL", "wss://api.mainnet-beta.solana.com");
        env::set_var("HELIUS_API_KEY", "dummy-key");
        env::set_var(
            "HELIUS_RPC_URL",
            "https://mainnet.helius-rpc.com/?api-key=dummy",
        );
        env::set_var(
            "DATABASE_URL",
            "postgresql://postgres:password@localhost:5432/sonar",
        );
        env::set_var("REDIS_URL", "redis://localhost:6379");
        env::set_var("SP1_PROVING_KEY", "/tmp/sp1.key");
        env::set_var("GROTH16_PARAMS", "/tmp/groth16.params");

        // Resolve path relative to workspace root (where Cargo.toml lives).
        let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
        // crates/common → workspace root is two levels up.
        let root = std::path::PathBuf::from(&manifest)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        let path = root.join("config/default.toml");

        let cfg = Config::load(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.network.chain_id, "mainnet");
        assert_eq!(cfg.observability.log_level, "info");
        assert_eq!(cfg.observability.metrics_port, 9090);
        assert!(!cfg.prover.mock_prover);

        // Cleanup.
        for var in &[
            "SOLANA_RPC_URL",
            "SOLANA_WS_URL",
            "HELIUS_API_KEY",
            "HELIUS_RPC_URL",
            "DATABASE_URL",
            "REDIS_URL",
            "SP1_PROVING_KEY",
            "GROTH16_PARAMS",
        ] {
            env::remove_var(var);
        }
    }

    #[test]
    fn test_devnet_toml_loads_with_env() {
        env::set_var("SOLANA_RPC_URL", "https://api.devnet.solana.com");
        env::set_var("SOLANA_WS_URL", "wss://api.devnet.solana.com");
        env::set_var("HELIUS_API_KEY", "dummy-devnet-key");
        env::set_var(
            "HELIUS_RPC_URL",
            "https://devnet.helius-rpc.com/?api-key=dummy-devnet-key",
        );
        env::set_var(
            "DATABASE_URL",
            "postgresql://postgres:password@localhost:5432/sonar_devnet",
        );
        env::set_var("REDIS_URL", "redis://localhost:6379");
        env::set_var("SP1_PROVING_KEY", "/tmp/sp1-devnet.key");
        env::set_var("GROTH16_PARAMS", "/tmp/groth16-devnet.params");

        let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
        let root = std::path::PathBuf::from(&manifest)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        let path = root.join("config/devnet.toml");

        let cfg = Config::load(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.network.chain_id, "devnet");
        assert_eq!(cfg.indexer.http_port, 8080);
        assert_eq!(cfg.coordinator.indexer_url, "http://localhost:8080");
        assert!(!cfg.prover.mock_prover);

        for var in &[
            "SOLANA_RPC_URL",
            "SOLANA_WS_URL",
            "HELIUS_API_KEY",
            "HELIUS_RPC_URL",
            "DATABASE_URL",
            "REDIS_URL",
            "SP1_PROVING_KEY",
            "GROTH16_PARAMS",
        ] {
            env::remove_var(var);
        }
    }
}
