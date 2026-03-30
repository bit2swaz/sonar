use std::{fs, path::Path, sync::RwLock};

use clone_agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoV2,
    ReplicaAccountInfoV3, ReplicaAccountInfoVersions, Result as PluginResult, SlotStatus,
};
use serde::Deserialize;
use solana_sdk::pubkey::Pubkey;

const PLUGIN_NAME: &str = "sonar-geyser-plugin";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct IndexerPluginConfig {
    pub libpath: String,
    #[serde(default)]
    pub log_level: Option<String>,
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
}

#[derive(Debug, Clone, Default)]
struct PluginState {
    config: Option<IndexerPluginConfig>,
    loaded: bool,
}

#[derive(Debug, Default)]
pub struct SonarGeyserPlugin {
    state: RwLock<PluginState>,
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
        let state = self.state.read().expect("plugin state lock poisoned");
        f(&state)
    }

    fn with_state_mut<T>(&self, f: impl FnOnce(&mut PluginState) -> T) -> T {
        let mut state = self.state.write().expect("plugin state lock poisoned");
        f(&mut state)
    }

    fn record_from_update(account: ReplicaAccountInfoVersions<'_>) -> AccountUpdateRecord {
        match account {
            ReplicaAccountInfoVersions::V0_0_1(info) => Self::record_from_v1(info),
            ReplicaAccountInfoVersions::V0_0_2(info) => Self::record_from_v2(info),
            ReplicaAccountInfoVersions::V0_0_3(info) => Self::record_from_v3(info),
        }
    }

    fn record_from_v1(info: &ReplicaAccountInfo<'_>) -> AccountUpdateRecord {
        AccountUpdateRecord {
            pubkey: format_pubkey(info.pubkey),
            owner: format_pubkey(info.owner),
            lamports: info.lamports,
            executable: info.executable,
            rent_epoch: info.rent_epoch,
            data_len: info.data.len(),
            write_version: info.write_version,
            txn_signature: None,
        }
    }

    fn record_from_v2(info: &ReplicaAccountInfoV2<'_>) -> AccountUpdateRecord {
        AccountUpdateRecord {
            pubkey: format_pubkey(info.pubkey),
            owner: format_pubkey(info.owner),
            lamports: info.lamports,
            executable: info.executable,
            rent_epoch: info.rent_epoch,
            data_len: info.data.len(),
            write_version: info.write_version,
            txn_signature: info.txn_signature.map(ToString::to_string),
        }
    }

    fn record_from_v3(info: &ReplicaAccountInfoV3<'_>) -> AccountUpdateRecord {
        let txn_signature = info
            .txn
            .and_then(|txn| txn.signatures().first())
            .map(ToString::to_string);

        AccountUpdateRecord {
            pubkey: format_pubkey(info.pubkey),
            owner: format_pubkey(info.owner),
            lamports: info.lamports,
            executable: info.executable,
            rent_epoch: info.rent_epoch,
            data_len: info.data.len(),
            write_version: info.write_version,
            txn_signature,
        }
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

        self.with_state_mut(|state| {
            state.config = Some(config.clone());
            state.loaded = true;
        });

        log::info!(
            target: "sonar::geyser",
            "loaded {} (reload={}, libpath={}, log_level={})",
            self.name(),
            is_reload,
            config.libpath,
            config.log_level.as_deref().unwrap_or("validator"),
        );

        Ok(())
    }

    fn on_unload(&mut self) {
        self.with_state_mut(|state| {
            state.loaded = false;
        });

        log::info!(target: "sonar::geyser", "unloaded {}", self.name());
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions<'_>,
        slot: u64,
        is_startup: bool,
    ) -> PluginResult<()> {
        let record = Self::record_from_update(account);

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
        log::info!(target: "sonar::geyser", "startup account replay completed");
        Ok(())
    }

    fn update_slot_status(
        &self,
        slot: u64,
        parent: Option<u64>,
        status: &SlotStatus,
    ) -> PluginResult<()> {
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

fn format_pubkey(bytes: &[u8]) -> String {
    Pubkey::try_from(bytes)
        .map(|pubkey| pubkey.to_string())
        .unwrap_or_else(|_| format!("invalid-pubkey-len-{}", bytes.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clone_agave_geyser_plugin_interface::geyser_plugin_interface::{
        ReplicaAccountInfo, ReplicaAccountInfoVersions,
    };

    fn sample_config_json() -> String {
        serde_json::json!({
            "libpath": "/tmp/libsonar_geyser.so",
            "log_level": "info"
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
        let mut plugin = SonarGeyserPlugin::default();

        plugin.on_load(&sample_config_json(), false).unwrap();

        let state = plugin.with_state(|state| state.clone());
        assert!(state.loaded);
        assert_eq!(
            state.config,
            Some(IndexerPluginConfig {
                libpath: "/tmp/libsonar_geyser.so".to_string(),
                log_level: Some("info".to_string()),
            })
        );
    }

    #[test]
    fn test_update_account_with_dummy_data_does_not_panic() {
        let plugin = SonarGeyserPlugin::default();
        let account = sample_replica_account();

        plugin
            .update_account(ReplicaAccountInfoVersions::V0_0_1(&account), 123, false)
            .unwrap();
    }

    #[test]
    fn test_record_from_update_extracts_expected_fields() {
        let account = sample_replica_account();
        let record =
            SonarGeyserPlugin::record_from_update(ReplicaAccountInfoVersions::V0_0_1(&account));

        assert_eq!(record.lamports, 42);
        assert_eq!(record.data_len, 4);
        assert_eq!(record.write_version, 99);
        assert_eq!(record.txn_signature, None);
    }

    #[test]
    fn test_on_unload_marks_plugin_not_loaded() {
        let mut plugin = SonarGeyserPlugin::default();
        plugin.on_load(&sample_config_json(), false).unwrap();

        plugin.on_unload();

        assert!(!plugin.with_state(|state| state.loaded));
    }
}
