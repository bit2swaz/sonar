use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use solana_sdk::{hash::hash, pubkey::Pubkey};
use sqlx::{
    migrate::Migrator,
    postgres::PgRow,
    postgres::{PgPoolOptions, Postgres},
    PgPool, QueryBuilder, Row,
};

static MIGRATOR: Migrator = sqlx::migrate!();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub database_url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountUpdate {
    pub slot: u64,
    pub pubkey: Pubkey,
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data_hash: [u8; 32],
    pub write_version: u64,
}

impl AccountUpdate {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        slot: u64,
        pubkey: Pubkey,
        lamports: u64,
        owner: Pubkey,
        executable: bool,
        rent_epoch: u64,
        data: &[u8],
        write_version: u64,
    ) -> Self {
        Self {
            slot,
            pubkey,
            lamports,
            owner,
            executable,
            rent_epoch,
            data_hash: hash(data).to_bytes(),
            write_version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountState {
    pub slot: u64,
    pub pubkey: Pubkey,
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data_hash: [u8; 32],
    pub write_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotUpdate {
    pub slot: u64,
    pub parent_slot: Option<u64>,
    pub status: String,
}

#[derive(Debug, Clone)]
struct AccountStateRow {
    slot: i64,
    pubkey: Vec<u8>,
    lamports: i64,
    owner: Vec<u8>,
    executable: bool,
    rent_epoch: i64,
    data_hash: Vec<u8>,
    write_version: String,
}

impl<'r> sqlx::FromRow<'r, PgRow> for AccountStateRow {
    fn from_row(row: &'r PgRow) -> std::result::Result<Self, sqlx::Error> {
        Ok(Self {
            slot: row.try_get("slot")?,
            pubkey: row.try_get("pubkey")?,
            lamports: row.try_get("lamports")?,
            owner: row.try_get("owner")?,
            executable: row.try_get("executable")?,
            rent_epoch: row.try_get("rent_epoch")?,
            data_hash: row.try_get("data_hash")?,
            write_version: row.try_get("write_version")?,
        })
    }
}

pub async fn connect_pool(config: &DatabaseConfig) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await
        .with_context(|| format!("failed to connect to postgres at {}", config.database_url))
}

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("failed to run indexer migrations")
}

pub async fn insert_account_batch(pool: &PgPool, updates: &[AccountUpdate]) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let normalized_updates = updates
        .iter()
        .map(|update| {
            Ok((
                to_i64("slot", update.slot)?,
                update.pubkey.to_bytes().to_vec(),
                to_i64("lamports", update.lamports)?,
                update.owner.to_bytes().to_vec(),
                update.executable,
                u64_as_i64(update.rent_epoch),
                update.data_hash.to_vec(),
                format_write_version(update.write_version),
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut query_builder: QueryBuilder<Postgres> = QueryBuilder::new(
		"INSERT INTO account_history (slot, pubkey, lamports, owner, executable, rent_epoch, data_hash, write_version) ",
	);

    query_builder.push_values(normalized_updates, |mut row, update| {
        row.push_bind(update.0)
            .push_bind(update.1)
            .push_bind(update.2)
            .push_bind(update.3)
            .push_bind(update.4)
            .push_bind(update.5)
            .push_bind(update.6)
            .push_bind(update.7);
    });

    query_builder.push(
        " ON CONFLICT (slot, pubkey, write_version) DO UPDATE SET \
		 lamports = EXCLUDED.lamports, \
		 owner = EXCLUDED.owner, \
		 executable = EXCLUDED.executable, \
		 rent_epoch = EXCLUDED.rent_epoch, \
		 data_hash = EXCLUDED.data_hash",
    );

    query_builder
        .build()
        .persistent(false)
        .execute(pool)
        .await
        .context("failed to insert account history batch")?;

    Ok(())
}

pub async fn insert_slot_update(pool: &PgPool, update: &SlotUpdate) -> Result<()> {
    sqlx::query(
        "INSERT INTO slot_metadata (slot, blockhash, parent_slot, timestamp, status) \
		 VALUES ($1, NULL, $2, NULL, $3) \
		 ON CONFLICT (slot) DO UPDATE SET \
		 parent_slot = EXCLUDED.parent_slot, \
		 status = EXCLUDED.status",
    )
    .bind(to_i64("slot", update.slot)?)
    .bind(
        update
            .parent_slot
            .map(|parent_slot| to_i64("parent_slot", parent_slot))
            .transpose()?,
    )
    .bind(&update.status)
    .execute(pool)
    .await
    .context("failed to upsert slot metadata")?;

    Ok(())
}

pub async fn query_account_history(
    pool: &PgPool,
    pubkey: &Pubkey,
    from_slot: u64,
    to_slot: u64,
) -> Result<Vec<AccountState>> {
    let rows = sqlx::query_as::<_, AccountStateRow>(
        "SELECT slot, pubkey, lamports, owner, executable, rent_epoch, data_hash, write_version \
		 FROM account_history \
		 WHERE pubkey = $1 AND slot BETWEEN $2 AND $3 \
		 ORDER BY slot ASC, write_version ASC",
    )
    .bind(pubkey.to_bytes().to_vec())
    .bind(to_i64("from_slot", from_slot)?)
    .bind(to_i64("to_slot", to_slot)?)
    .fetch_all(pool)
    .await
    .context("failed to query account history")?;

    rows.into_iter().map(AccountState::try_from).collect()
}

pub async fn query_account_snapshot(
    pool: &PgPool,
    pubkey: &Pubkey,
    slot: u64,
) -> Result<Option<AccountState>> {
    let row = sqlx::query_as::<_, AccountStateRow>(
        "SELECT slot, pubkey, lamports, owner, executable, rent_epoch, data_hash, write_version \
		 FROM account_history \
		 WHERE pubkey = $1 AND slot <= $2 \
		 ORDER BY slot DESC, write_version DESC \
		 LIMIT 1",
    )
    .bind(pubkey.to_bytes().to_vec())
    .bind(to_i64("slot", slot)?)
    .fetch_optional(pool)
    .await
    .context("failed to query account snapshot")?;

    row.map(AccountState::try_from).transpose()
}

/// Return the ordered list of lamport balances for `pubkey` in the inclusive
/// slot range `[from_slot, to_slot]`.  Each row represents the balance at
/// that particular write — multiple writes per slot are preserved in
/// write_version order.
pub async fn query_balances_in_range(
    pool: &PgPool,
    pubkey: &[u8; 32],
    from_slot: u64,
    to_slot: u64,
) -> Result<Vec<u64>> {
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT lamports \
         FROM account_history \
         WHERE pubkey = $1 AND slot BETWEEN $2 AND $3 \
         ORDER BY slot ASC, write_version ASC",
    )
    .bind(pubkey.to_vec())
    .bind(to_i64("from_slot", from_slot)?)
    .bind(to_i64("to_slot", to_slot)?)
    .fetch_all(pool)
    .await
    .context("failed to query balances in range")?;

    rows.into_iter().map(|v| to_u64("lamports", v)).collect()
}

impl TryFrom<AccountStateRow> for AccountState {
    type Error = anyhow::Error;

    fn try_from(row: AccountStateRow) -> Result<Self> {
        Ok(Self {
            slot: to_u64("slot", row.slot)?,
            pubkey: pubkey_from_vec("pubkey", row.pubkey)?,
            lamports: to_u64("lamports", row.lamports)?,
            owner: pubkey_from_vec("owner", row.owner)?,
            executable: row.executable,
            rent_epoch: i64_as_u64(row.rent_epoch),
            data_hash: bytes32_from_vec("data_hash", row.data_hash)?,
            write_version: parse_write_version(&row.write_version)?,
        })
    }
}

fn to_i64(field: &str, value: u64) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} overflowed postgres BIGINT"))
}

fn to_u64(field: &str, value: i64) -> Result<u64> {
    u64::try_from(value).with_context(|| format!("{field} was negative in postgres row"))
}

/// Bitwise-cast a `u64` to `i64` without bounds-checking.  Used for values
/// such as `rent_epoch` that Solana assigns `u64::MAX` to for rent-exempt
/// accounts — which would otherwise overflow a signed BIGINT check.
fn u64_as_i64(value: u64) -> i64 {
    i64::from_ne_bytes(value.to_ne_bytes())
}

/// Inverse of [`u64_as_i64`]: reconstruct the original `u64` bit-pattern.
fn i64_as_u64(value: i64) -> u64 {
    u64::from_ne_bytes(value.to_ne_bytes())
}

fn format_write_version(value: u64) -> String {
    format!("{value:020}")
}

fn parse_write_version(value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("write_version '{value}' was not a valid u64"))
}

fn bytes32_from_vec(field: &str, bytes: Vec<u8>) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_: Vec<u8>| anyhow!("{field} must contain exactly 32 bytes"))
}

fn pubkey_from_vec(field: &str, bytes: Vec<u8>) -> Result<Pubkey> {
    let bytes = bytes32_from_vec(field, bytes)?;
    Ok(Pubkey::new_from_array(bytes))
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        process::Command,
        thread::sleep,
        time::{Duration, Instant},
    };

    use super::*;

    struct DockerPostgres {
        container_id: String,
    }

    impl DockerPostgres {
        fn start() -> Result<(Self, String)> {
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
                bail!(
                    "docker run failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
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
                bail!(
                    "docker port failed: {}",
                    String::from_utf8_lossy(&port_output.stderr).trim()
                );
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

        fn wait_until_ready(&self) -> Result<()> {
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

            bail!("postgres container did not become ready in time")
        }
    }

    impl Drop for DockerPostgres {
        fn drop(&mut self) {
            let _ = Command::new("docker")
                .args(["rm", "-f", &self.container_id])
                .status();
        }
    }

    async fn with_test_pool<T, F, Fut>(test: F) -> Result<T>
    where
        F: FnOnce(PgPool) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let (_container, database_url) = DockerPostgres::start()?;

        let pool = connect_test_pool(&database_url, 5).await?;
        run_migrations(&pool).await?;

        test(pool).await
    }

    async fn connect_test_pool(database_url: &str, max_connections: u32) -> Result<PgPool> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut last_error = None;

        while Instant::now() < deadline {
            match connect_pool(&DatabaseConfig {
                database_url: database_url.to_string(),
                max_connections,
            })
            .await
            {
                Ok(pool) => return Ok(pool),
                Err(error) => {
                    last_error = Some(error);
                    sleep(Duration::from_millis(250));
                },
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow!("postgres test container did not accept connections in time")
        }))
    }

    fn sample_update(slot: u64, lamports: u64, write_version: u64) -> AccountUpdate {
        AccountUpdate::new(
            slot,
            Pubkey::new_from_array([7; 32]),
            lamports,
            Pubkey::new_from_array([9; 32]),
            false,
            42,
            &[1, 2, 3, slot as u8],
            write_version,
        )
    }

    #[tokio::test]
    async fn test_migrations_create_expected_tables_and_indexes() -> Result<()> {
        with_test_pool(|pool| async move {
            let mut table_names = sqlx::query_scalar::<_, String>(
                "SELECT table_name \
				 FROM information_schema.tables \
				 WHERE table_schema = 'public' \
				 ORDER BY table_name",
            )
            .fetch_all(&pool)
            .await?;

            table_names.sort();
            assert_eq!(
                table_names,
                vec![
                    "_sqlx_migrations".to_string(),
                    "account_history".to_string(),
                    "request_tracking".to_string(),
                    "slot_metadata".to_string(),
                ]
            );

            let index_rows = sqlx::query_as::<_, (String, String)>(
                "SELECT indexname, indexdef \
                 FROM pg_indexes \
                 WHERE schemaname = 'public' AND tablename = 'account_history' \
                 ORDER BY indexname",
            )
            .fetch_all(&pool)
            .await?;

            let index_names = index_rows
                .iter()
                .map(|(index_name, _)| index_name.clone())
                .collect::<Vec<_>>();
            assert!(
                index_names.contains(&"account_history_pubkey_slot_desc_idx".to_string()),
                "expected compound pubkey/slot desc index to exist"
            );
            assert!(
                index_names.contains(&"account_history_slot_idx".to_string()),
                "expected slot index to exist"
            );
            assert!(
                !index_names.contains(&"account_history_pubkey_slot_idx".to_string()),
                "legacy wider compound index should be removed by follow-up migration"
            );

            let compound_index = index_rows
                .iter()
                .find(|(index_name, _)| index_name == "account_history_pubkey_slot_desc_idx")
                .map(|(_, index_def)| index_def)
                .context("missing account_history_pubkey_slot_desc_idx definition")?;
            assert!(
                compound_index.contains("(pubkey, slot DESC)"),
                "expected compound index definition to target (pubkey, slot DESC), got: {compound_index}"
            );

            let applied_versions = sqlx::query_scalar::<_, i64>(
                "SELECT version FROM _sqlx_migrations ORDER BY version",
            )
            .fetch_all(&pool)
            .await?;
            assert_eq!(applied_versions, vec![202603310001_i64, 202604050000_i64]);

            run_migrations(&pool).await?;

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn test_connect_pool_rejects_invalid_database_url() {
        let error = connect_pool(&DatabaseConfig {
            database_url: "not-a-valid-postgres-url".to_string(),
            max_connections: 1,
        })
        .await
        .expect_err("invalid database URL should fail");

        assert!(
            error.to_string().contains("failed to connect to postgres"),
            "unexpected error: {error:#}"
        );
    }

    #[tokio::test]
    async fn test_insert_and_query_account_history() -> Result<()> {
        with_test_pool(|pool| async move {
            let mut updates = vec![
                sample_update(10, 100, 1),
                sample_update(20, 200, 2),
                sample_update(30, 300, 3),
            ];
            updates.push(AccountUpdate::new(
                15,
                Pubkey::new_from_array([5; 32]),
                999,
                Pubkey::new_from_array([6; 32]),
                true,
                99,
                &[8, 8, 8],
                1,
            ));

            insert_account_batch(&pool, &updates).await?;

            let history =
                query_account_history(&pool, &Pubkey::new_from_array([7; 32]), 0, 50).await?;
            assert_eq!(history.len(), 3);
            assert_eq!(history[0].slot, 10);
            assert_eq!(history[1].lamports, 200);
            assert_eq!(history[2].write_version, 3);

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn test_query_account_snapshot_returns_latest_state() -> Result<()> {
        with_test_pool(|pool| async move {
            insert_account_batch(
                &pool,
                &[
                    sample_update(10, 100, 1),
                    sample_update(20, 250, 2),
                    sample_update(30, 400, 3),
                ],
            )
            .await?;

            let snapshot = query_account_snapshot(&pool, &Pubkey::new_from_array([7; 32]), 25)
                .await?
                .expect("snapshot should exist");
            assert_eq!(snapshot.slot, 20);
            assert_eq!(snapshot.lamports, 250);

            let missing =
                query_account_snapshot(&pool, &Pubkey::new_from_array([7; 32]), 5).await?;
            assert!(missing.is_none());

            Ok(())
        })
        .await
    }
}
