use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result as AnyhowResult};
use clone_agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoV2,
    ReplicaAccountInfoV3, ReplicaAccountInfoVersions, Result as PluginResult, SlotStatus,
};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;
use tokio::runtime::{Builder, Runtime};

use crate::db::{self, AccountUpdate, DatabaseConfig, SlotUpdate};

const PLUGIN_NAME: &str = "sonar-geyser-plugin";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IndexerPluginConfig {
    pub libpath: String,
    pub database_url: String,
    #[serde(default)]
    pub log_level: Option<String>,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AccountUpdateRecord {
    pub pubkey: String,
    pub owner: String,
    pub lamports: u64,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data_len: usize,
    pub write_version: u64,
    pub txn_signature: Option<String>,
    pub persisted_update: AccountUpdate,
}

struct DatabaseWriter {
    runtime: Runtime,
    pool: sqlx::PgPool,
    operation_lock: Mutex<()>,
}

impl std::fmt::Debug for DatabaseWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseWriter").finish_non_exhaustive()
    }
}

impl DatabaseWriter {
    fn connect(config: &IndexerPluginConfig) -> PluginResult<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("sonar-geyser-db")
            .build()
            .map_err(plugin_custom_error)?;

        let db_config = DatabaseConfig {
            database_url: config.database_url.clone(),
            max_connections: config.max_connections,
        };

        let pool = runtime
            .block_on(async {
                let pool = db::connect_pool(&db_config).await?;
                db::run_migrations(&pool).await?;
                AnyhowResult::<_>::Ok(pool)
            })
            .map_err(plugin_custom_error)?;

        Ok(Self {
            runtime,
            pool,
            operation_lock: Mutex::new(()),
        })
    }

    fn flush_account_batch(&self, updates: &[AccountUpdate]) -> PluginResult<()> {
        let _guard = self
            .operation_lock
            .lock()
            .expect("db operation lock poisoned");
        self.runtime
            .block_on(db::insert_account_batch(&self.pool, updates))
            .map_err(|error| GeyserPluginError::AccountsUpdateError {
                msg: error.to_string(),
            })
    }

    fn write_slot_update(&self, update: &SlotUpdate) -> PluginResult<()> {
        let _guard = self
            .operation_lock
            .lock()
            .expect("db operation lock poisoned");
        self.runtime
            .block_on(db::insert_slot_update(&self.pool, update))
            .map_err(|error| GeyserPluginError::SlotStatusUpdateError {
                msg: error.to_string(),
            })
    }

    #[cfg(test)]
    fn pool(&self) -> sqlx::PgPool {
        self.pool.clone()
    }
}

#[derive(Debug, Default)]
struct PluginState {
    config: Option<IndexerPluginConfig>,
    loaded: bool,
    database: Option<Arc<DatabaseWriter>>,
    pending_updates: Vec<AccountUpdate>,
}

#[derive(Debug, Default)]
pub struct SonarGeyserPlugin {
    state: Mutex<PluginState>,
}

impl SonarGeyserPlugin {
    fn parse_config(config_input: &str) -> PluginResult<IndexerPluginConfig> {
        match serde_json::from_str::<IndexerPluginConfig>(config_input) {
            Ok(config) => Ok(config),
            Err(initial_error) => {
                let path = Path::new(config_input);
                if !path.exists() {
                    return Err(GeyserPluginError::ConfigFileReadError {
                        msg: initial_error.to_string(),
                    });
                }

                let raw = fs::read_to_string(path)?;
                serde_json::from_str::<IndexerPluginConfig>(&raw).map_err(|parse_error| {
                    GeyserPluginError::ConfigFileReadError {
                        msg: parse_error.to_string(),
                    }
                })
            },
        }
    }

    fn with_state<T>(&self, f: impl FnOnce(&PluginState) -> T) -> T {
        let state = self.state.lock().expect("plugin state lock poisoned");
        f(&state)
    }

    fn with_state_mut<T>(&self, f: impl FnOnce(&mut PluginState) -> T) -> T {
        let mut state = self.state.lock().expect("plugin state lock poisoned");
        f(&mut state)
    }

    fn record_from_update(
        slot: u64,
        account: ReplicaAccountInfoVersions<'_>,
    ) -> AnyhowResult<AccountUpdateRecord> {
        match account {
            ReplicaAccountInfoVersions::V0_0_1(info) => Self::record_from_v1(slot, info),
            ReplicaAccountInfoVersions::V0_0_2(info) => Self::record_from_v2(slot, info),
            ReplicaAccountInfoVersions::V0_0_3(info) => Self::record_from_v3(slot, info),
        }
    }

    fn record_from_v1(
        slot: u64,
        info: &ReplicaAccountInfo<'_>,
    ) -> AnyhowResult<AccountUpdateRecord> {
        let pubkey = pubkey_from_bytes(info.pubkey)?;
        let owner = pubkey_from_bytes(info.owner)?;

        Ok(AccountUpdateRecord {
            pubkey: pubkey.to_string(),
            owner: owner.to_string(),
            lamports: info.lamports,
            executable: info.executable,
            rent_epoch: info.rent_epoch,
            data_len: info.data.len(),
            write_version: info.write_version,
            txn_signature: None,
            persisted_update: AccountUpdate::new(
                slot,
                pubkey,
                info.lamports,
                owner,
                info.executable,
                info.rent_epoch,
                info.data,
                info.write_version,
            ),
        })
    }

    fn record_from_v2(
        slot: u64,
        info: &ReplicaAccountInfoV2<'_>,
    ) -> AnyhowResult<AccountUpdateRecord> {
        let pubkey = pubkey_from_bytes(info.pubkey)?;
        let owner = pubkey_from_bytes(info.owner)?;

        Ok(AccountUpdateRecord {
            pubkey: pubkey.to_string(),
            owner: owner.to_string(),
            lamports: info.lamports,
            executable: info.executable,
            rent_epoch: info.rent_epoch,
            data_len: info.data.len(),
            write_version: info.write_version,
            txn_signature: info.txn_signature.map(ToString::to_string),
            persisted_update: AccountUpdate::new(
                slot,
                pubkey,
                info.lamports,
                owner,
                info.executable,
                info.rent_epoch,
                info.data,
                info.write_version,
            ),
        })
    }

    fn record_from_v3(
        slot: u64,
        info: &ReplicaAccountInfoV3<'_>,
    ) -> AnyhowResult<AccountUpdateRecord> {
        let txn_signature = info
            .txn
            .and_then(|txn| txn.signatures().first())
            .map(ToString::to_string);

        let pubkey = pubkey_from_bytes(info.pubkey)?;
        let owner = pubkey_from_bytes(info.owner)?;

        Ok(AccountUpdateRecord {
            pubkey: pubkey.to_string(),
            owner: owner.to_string(),
            lamports: info.lamports,
            executable: info.executable,
            rent_epoch: info.rent_epoch,
            data_len: info.data.len(),
            write_version: info.write_version,
            txn_signature,
            persisted_update: AccountUpdate::new(
                slot,
                pubkey,
                info.lamports,
                owner,
                info.executable,
                info.rent_epoch,
                info.data,
                info.write_version,
            ),
        })
    }

    fn flush_pending_updates(&self) -> PluginResult<()> {
        let Some((database, batch)) = self.take_pending_updates()? else {
            return Ok(());
        };

        database.flush_account_batch(&batch)
    }

    fn take_pending_updates(
        &self,
    ) -> PluginResult<Option<(Arc<DatabaseWriter>, Vec<AccountUpdate>)>> {
        let mut state = self.state.lock().expect("plugin state lock poisoned");
        if state.pending_updates.is_empty() {
            return Ok(None);
        }

        let database =
            state
                .database
                .clone()
                .ok_or_else(|| GeyserPluginError::AccountsUpdateError {
                    msg: "plugin database has not been initialized".to_string(),
                })?;

        let batch = std::mem::take(&mut state.pending_updates);
        Ok(Some((database, batch)))
    }
}

impl GeyserPlugin for SonarGeyserPlugin {
    fn setup_logger(
        &self,
        logger: &'static dyn log::Log,
        level: log::LevelFilter,
    ) -> PluginResult<()> {
        let _ = log::set_logger(logger);
        log::set_max_level(level);
        Ok(())
    }

    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn on_load(&mut self, config_file: &str, is_reload: bool) -> PluginResult<()> {
        let config = Self::parse_config(config_file)?;
        let database = Arc::new(DatabaseWriter::connect(&config)?);

        self.with_state_mut(|state| {
            state.config = Some(config.clone());
            state.loaded = true;
            state.database = Some(database);
            state.pending_updates.clear();
        });

        log::info!(
            target: "sonar::geyser",
            "loaded {} (reload={}, libpath={}, max_connections={}, batch_size={}, log_level={})",
            self.name(),
            is_reload,
            config.libpath,
            config.max_connections,
            config.batch_size,
            config.log_level.as_deref().unwrap_or("validator"),
        );

        Ok(())
    }

    fn on_unload(&mut self) {
        if let Err(error) = self.flush_pending_updates() {
            log::error!(target: "sonar::geyser", "failed to flush pending account updates during unload: {error}");
        }

        self.with_state_mut(|state| {
            state.loaded = false;
            state.database = None;
        });

        log::info!(target: "sonar::geyser", "unloaded {}", self.name());
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions<'_>,
        slot: u64,
        is_startup: bool,
    ) -> PluginResult<()> {
        let record = Self::record_from_update(slot, account).map_err(|error| {
            GeyserPluginError::AccountsUpdateError {
                msg: error.to_string(),
            }
        })?;

        let should_flush = self.with_state_mut(|state| {
            state.pending_updates.push(record.persisted_update.clone());
            let batch_size = state
                .config
                .as_ref()
                .map(|config| config.batch_size)
                .unwrap_or(1);
            state.pending_updates.len() >= batch_size
        });

        if should_flush {
            self.flush_pending_updates()?;
        }

        log::info!(
            target: "sonar::geyser",
            "account update slot={} startup={} pubkey={} owner={} lamports={} executable={} rent_epoch={} data_len={} write_version={} txn_signature={}",
            slot,
            is_startup,
            record.pubkey,
            record.owner,
            record.lamports,
            record.executable,
            record.rent_epoch,
            record.data_len,
            record.write_version,
            record.txn_signature.as_deref().unwrap_or("none"),
        );

        Ok(())
    }

    fn notify_end_of_startup(&self) -> PluginResult<()> {
        self.flush_pending_updates()?;
        log::info!(target: "sonar::geyser", "startup account replay completed");
        Ok(())
    }

    fn update_slot_status(
        &self,
        slot: u64,
        parent: Option<u64>,
        status: &SlotStatus,
    ) -> PluginResult<()> {
        let database = self.with_state(|state| state.database.clone());
        if let Some(database) = database {
            database.write_slot_update(&SlotUpdate {
                slot,
                parent_slot: parent,
                status: status.as_str().to_string(),
            })?;
        }

        log::debug!(
            target: "sonar::geyser",
            "slot update slot={} parent={:?} status={}",
            slot,
            parent,
            status.as_str(),
        );

        Ok(())
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }

    fn account_data_snapshot_notifications_enabled(&self) -> bool {
        true
    }
}

fn default_max_connections() -> u32 {
    5
}

fn default_batch_size() -> usize {
    1
}

fn plugin_custom_error(error: impl Into<anyhow::Error>) -> GeyserPluginError {
    let message = error.into().to_string();
    GeyserPluginError::Custom(Box::new(std::io::Error::other(message)))
}

fn pubkey_from_bytes(bytes: &[u8]) -> AnyhowResult<Pubkey> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_: std::array::TryFromSliceError| {
            anyhow!("expected 32-byte pubkey, got {} bytes", bytes.len())
        })?;
    Ok(Pubkey::new_from_array(bytes))
}

#[cfg(test)]
mod tests {
    use std::{
        process::Command,
        thread::sleep,
        time::{Duration, Instant},
    };

    use super::*;
    use clone_agave_geyser_plugin_interface::geyser_plugin_interface::{
        ReplicaAccountInfo, ReplicaAccountInfoVersions,
    };

    struct DockerPostgres {
        container_id: String,
    }

    impl DockerPostgres {
        fn start() -> AnyhowResult<(Self, String)> {
            let output = Command::new("docker")
                .args([
                    "run",
                    "--rm",
                    "-d",
                    "-e",
                    "POSTGRES_PASSWORD=postgres",
                    "-e",
                    "POSTGRES_DB=postgres",
                    "-P",
                    "postgres:16-alpine",
                ])
                .output()
                .context("failed to start postgres docker container")?;

            if !output.status.success() {
                return Err(anyhow!(
                    "docker run failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }

            let container_id = String::from_utf8(output.stdout)
                .context("docker output was not valid utf8")?
                .trim()
                .to_string();

            let port_output = Command::new("docker")
                .args(["port", &container_id, "5432/tcp"])
                .output()
                .context("failed to query postgres container port")?;

            if !port_output.status.success() {
                return Err(anyhow!(
                    "docker port failed: {}",
                    String::from_utf8_lossy(&port_output.stderr).trim()
                ));
            }

            let port_line = String::from_utf8(port_output.stdout)
                .context("docker port output was not valid utf8")?
                .lines()
                .next()
                .context("docker port returned no mapped port")?
                .trim()
                .to_string();

            let host_port = port_line
                .rsplit(':')
                .next()
                .context("failed to parse docker mapped port")?;

            let container = Self { container_id };
            container.wait_until_ready()?;

            Ok((
                container,
                format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres"),
            ))
        }

        fn wait_until_ready(&self) -> AnyhowResult<()> {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                let output = Command::new("docker")
                    .args(["exec", &self.container_id, "pg_isready", "-U", "postgres"])
                    .output();

                if let Ok(output) = output {
                    if output.status.success() {
                        return Ok(());
                    }
                }

                sleep(Duration::from_millis(500));
            }

            Err(anyhow!("postgres container did not become ready in time"))
        }
    }

    impl Drop for DockerPostgres {
        fn drop(&mut self) {
            let _ = Command::new("docker")
                .args(["rm", "-f", &self.container_id])
                .status();
        }
    }

    fn with_database_url<T>(test: impl FnOnce(String) -> AnyhowResult<T>) -> AnyhowResult<T> {
        let (_container, database_url) = DockerPostgres::start()?;

        test(database_url)
    }

    fn sample_config_json(database_url: &str) -> String {
        serde_json::json!({
            "libpath": "/tmp/libsonar_geyser.so",
            "database_url": database_url,
            "log_level": "info",
            "max_connections": 4,
            "batch_size": 2
        })
        .to_string()
    }

    fn sample_replica_account() -> ReplicaAccountInfo<'static> {
        ReplicaAccountInfo {
            pubkey: &[7; 32],
            lamports: 42,
            owner: &[9; 32],
            executable: false,
            rent_epoch: 12,
            data: &[1, 2, 3, 4],
            write_version: 99,
        }
    }

    #[test]
    fn test_on_load_parses_inline_json_config() {
        let config_json = serde_json::json!({
            "libpath": "/tmp/libsonar_geyser.so",
            "database_url": "postgres://localhost/postgres",
            "log_level": "info",
            "max_connections": 4,
            "batch_size": 2
        })
        .to_string();

        let config = SonarGeyserPlugin::parse_config(&config_json).unwrap();

        assert_eq!(
            config,
            IndexerPluginConfig {
                libpath: "/tmp/libsonar_geyser.so".to_string(),
                database_url: "postgres://localhost/postgres".to_string(),
                log_level: Some("info".to_string()),
                max_connections: 4,
                batch_size: 2,
            }
        );
    }

    #[test]
    fn test_update_account_with_dummy_data_does_not_panic() {
        let plugin = SonarGeyserPlugin::default();
        let account = sample_replica_account();

        let result =
            plugin.update_account(ReplicaAccountInfoVersions::V0_0_1(&account), 123, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_record_from_update_extracts_expected_fields() {
        let account = sample_replica_account();
        let record = SonarGeyserPlugin::record_from_update(
            123,
            ReplicaAccountInfoVersions::V0_0_1(&account),
        )
        .unwrap();

        assert_eq!(record.lamports, 42);
        assert_eq!(record.data_len, 4);
        assert_eq!(record.write_version, 99);
        assert_eq!(record.txn_signature, None);
        assert_eq!(record.persisted_update.slot, 123);
    }

    #[test]
    fn test_on_unload_marks_plugin_not_loaded() {
        let mut plugin = SonarGeyserPlugin::default();
        plugin.with_state_mut(|state| state.loaded = true);

        plugin.on_unload();

        assert!(!plugin.with_state(|state| state.loaded));
    }

    #[test]
    fn test_update_account_persists_to_postgres() -> AnyhowResult<()> {
        with_database_url(|database_url| {
            let mut plugin = SonarGeyserPlugin::default();
            plugin.on_load(&sample_config_json(&database_url), false)?;

            let account = sample_replica_account();
            plugin.update_account(ReplicaAccountInfoVersions::V0_0_1(&account), 88, false)?;
            plugin.update_account(ReplicaAccountInfoVersions::V0_0_1(&account), 99, false)?;

            let pool = plugin
                .with_state(|state| state.database.as_ref().map(|database| database.pool()))
                .context("database pool should exist")?;

            let runtime = Builder::new_current_thread().enable_all().build()?;
            let snapshot = runtime
                .block_on(db::query_account_snapshot(
                    &pool,
                    &Pubkey::new_from_array([7; 32]),
                    100,
                ))?
                .context("snapshot should exist")?;
            assert_eq!(snapshot.slot, 99);
            assert_eq!(snapshot.lamports, 42);

            Ok(())
        })
    }
}
