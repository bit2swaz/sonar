#![allow(deprecated)]

use std::{
    fs::{self, File},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
    time::{Duration, Instant},
};

use anchor_lang::{AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use reqwest::StatusCode;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::{write_keypair_file, Keypair, Signer},
    system_instruction, system_program,
    transaction::Transaction,
};
use sonar_program::{
    accounts as sonar_accounts, instruction as sonar_instruction, RequestMetadata, RequestParams,
    RequestStatus, ResultAccount, HISTORICAL_AVG_COMPUTATION_ID,
};
use tempfile::TempDir;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    GenericImage, ImageExt,
};

const VALIDATOR_READY_TIMEOUT: Duration = Duration::from_secs(45);
const HTTP_READY_TIMEOUT: Duration = Duration::from_secs(45);
const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);
const CI_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const SEED_WAIT_TIMEOUT: Duration = Duration::from_secs(45);
const AIRDROP_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_FEE_LAMPORTS: u64 = 2_000_000;
const VALIDATOR_DYNAMIC_PORT_COUNT: u16 = 31;
const PORT_SCAN_START: u16 = 10_000;
const PORT_SCAN_END: u16 = 60_000;

fn callback_timeout() -> Duration {
    if let Ok(value) = std::env::var("SONAR_E2E_CALLBACK_TIMEOUT_SECONDS") {
        if let Ok(seconds) = value.parse::<u64>() {
            return Duration::from_secs(seconds.max(1));
        }
    }

    if std::env::var_os("CI").is_some() {
        CI_CALLBACK_TIMEOUT
    } else {
        DEFAULT_CALLBACK_TIMEOUT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BalancePoint {
    slot: u64,
    lamports: u64,
}

struct ChildGuard {
    name: &'static str,
    child: Child,
}

impl ChildGuard {
    fn spawn(name: &'static str, mut command: Command) -> Result<Self> {
        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn {name}"))?;
        Ok(Self { name, child })
    }

    fn check_running(&mut self) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            bail!("{name} exited early with status {status}", name = self.name);
        }
        Ok(())
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires docker, solana-test-validator, and local BPF program artifacts"]
async fn end_to_end_historical_average_flow_works() -> Result<()> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    ensure_local_artifacts(&repo_root)?;

    let workspace = TempDir::new().context("create e2e tempdir")?;
    let paths = TestPaths::new(&repo_root, workspace.path())?;

    let postgres = GenericImage::new("postgres", "16-alpine")
        .with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::message_on_stdout(
            "database system is ready to accept connections",
        ))
        .with_startup_timeout(Duration::from_secs(90))
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "postgres")
        .start()
        .await
        .context("start postgres testcontainer")?;
    let redis = GenericImage::new("redis", "7.2.4")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_startup_timeout(Duration::from_secs(60))
        .start()
        .await
        .context("start redis testcontainer")?;

    let postgres_url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        postgres.get_host().await?,
        postgres.get_host_port_ipv4(5432.tcp()).await?
    );
    let redis_url = format!(
        "redis://{}:{}",
        redis.get_host().await?,
        redis.get_host_port_ipv4(6379.tcp()).await?
    );

    let ports = PortLayout::allocate()?;
    write_plugin_config(&paths, &postgres_url)?;
    write_runtime_config(&paths, &postgres_url, &redis_url, &ports)?;
    write_historical_avg_verifier_registry_fixture(&paths)?;

    let mut validator = start_validator(&paths, &ports)?;
    wait_for_validator(
        &mut validator,
        &ports.rpc_url(),
        &paths.log_path("validator"),
    )
    .await?;

    let rpc = RpcClient::new_with_commitment(ports.rpc_url(), CommitmentConfig::confirmed());

    let coordinator = Keypair::new();
    let client = Keypair::new();
    write_keypair_file(&coordinator, &paths.coordinator_keypair)
        .map(|_| ())
        .map_err(|error| anyhow!(error.to_string()))
        .context("write coordinator keypair")?;
    airdrop(&rpc, &coordinator.pubkey(), 10 * LAMPORTS_PER_SOL)?;
    airdrop(&rpc, &client.pubkey(), 10 * LAMPORTS_PER_SOL)?;

    let mut indexer = start_indexer(&paths)?;
    wait_for_indexer(
        &mut indexer,
        &ports.indexer_url(),
        &paths.log_path("indexer"),
    )
    .await?;

    let observed = Keypair::new();
    let balances = seed_account_history(&rpc, &client, &observed).await?;
    let seeded_to_slot = rpc.get_slot()?;
    let expected_balances: Vec<u64> = balances.iter().map(|point| point.lamports).collect();
    let expected_avg = expected_balances.iter().sum::<u64>() / expected_balances.len() as u64;
    wait_for_indexed_balances(
        &ports.indexer_url(),
        &observed.pubkey(),
        0,
        seeded_to_slot,
        &expected_balances,
        &paths.log_path("validator"),
        &paths.log_path("indexer"),
    )
    .await?;

    let mut prover = start_prover(&paths)?;
    let mut coordinator_worker = start_coordinator(&paths)?;
    indexer.check_running()?;
    prover.check_running()?;
    coordinator_worker.check_running()?;

    // Wait for the coordinator's WebSocket subscription to be established before
    // submitting the request — otherwise the log event can arrive before the
    // listener is subscribed and the job is never dispatched.
    wait_for_coordinator_ready(&mut coordinator_worker, &paths.log_path("coordinator")).await?;

    let request_id = Keypair::new().pubkey().to_bytes();
    let result_account_pda = submit_historical_average_request(
        &rpc,
        &client,
        &observed.pubkey(),
        0,
        seeded_to_slot,
        request_id,
    )?;

    let (request_metadata_pda, _) =
        Pubkey::find_program_address(&[b"request", &request_id], &sonar_program::id());

    let result_account = wait_for_result_account(
        &rpc,
        result_account_pda,
        expected_avg,
        &paths.log_path("validator"),
        &paths.log_path("coordinator"),
        &paths.log_path("prover"),
    )
    .await?;
    assert!(
        result_account.is_set,
        "sonar result account should be written"
    );
    assert_eq!(decode_u64(&result_account.result)?, expected_avg);

    let request_metadata = read_anchor_account::<RequestMetadata>(&rpc, request_metadata_pda)?;
    assert!(matches!(request_metadata.status, RequestStatus::Completed));

    Ok(())
}

struct TestPaths {
    repo_root: PathBuf,
    log_dir: PathBuf,
    ledger_dir: PathBuf,
    historical_avg_verifier_registry_fixture: PathBuf,
    plugin_config: PathBuf,
    runtime_config: PathBuf,
    coordinator_keypair: PathBuf,
    target_dir: PathBuf,
}

impl TestPaths {
    fn new(repo_root: &Path, workspace_dir: &Path) -> Result<Self> {
        let log_dir = workspace_dir.join("logs");
        let ledger_dir = workspace_dir.join("ledger");
        fs::create_dir_all(&log_dir).context("create log dir")?;
        fs::create_dir_all(&ledger_dir).context("create ledger dir")?;
        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            log_dir,
            ledger_dir,
            historical_avg_verifier_registry_fixture: workspace_dir
                .join("historical-avg-verifier-registry.json"),
            plugin_config: workspace_dir.join("geyser-plugin.json"),
            runtime_config: workspace_dir.join("sonar-e2e.toml"),
            coordinator_keypair: workspace_dir.join("coordinator-keypair.json"),
            target_dir: repo_root.join("target"),
        })
    }

    fn plugin_library(&self) -> PathBuf {
        self.target_dir.join("debug/libsonar_indexer.so")
    }

    fn sonar_program_so(&self) -> PathBuf {
        self.target_dir.join("deploy/sonar_program.so")
    }

    fn echo_callback_so(&self) -> PathBuf {
        self.target_dir.join("deploy/echo_callback.so")
    }

    fn log_path(&self, name: &str) -> PathBuf {
        self.log_dir.join(format!("{name}.log"))
    }
}

struct PortLayout {
    rpc_port: u16,
    faucet_port: u16,
    dynamic_port_start: u16,
    dynamic_port_end: u16,
    indexer_http_port: u16,
}

impl PortLayout {
    fn allocate() -> Result<Self> {
        let (dynamic_port_start, dynamic_port_end) =
            find_available_port_range(VALIDATOR_DYNAMIC_PORT_COUNT)?;
        let mut reserved_ports = Vec::new();
        let excluded_ranges = [(dynamic_port_start, dynamic_port_end)];

        // Find an rpc_port such that rpc_port+1 (the WS port) is also free.
        let rpc_port = find_available_rpc_port(&excluded_ranges)?;
        // Reserve both rpc and ws ports so they are not reused.
        reserved_ports.push(rpc_port);
        reserved_ports.push(rpc_port + 1);

        let faucet_port = find_available_port(&reserved_ports, &excluded_ranges)?;
        reserved_ports.push(faucet_port);

        let indexer_http_port = find_available_port(&reserved_ports, &excluded_ranges)?;

        Ok(Self {
            rpc_port,
            faucet_port,
            dynamic_port_start,
            dynamic_port_end,
            indexer_http_port,
        })
    }

    fn rpc_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.rpc_port)
    }

    fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.rpc_port + 1)
    }

    fn indexer_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.indexer_http_port)
    }
}

fn ensure_local_artifacts(repo_root: &Path) -> Result<()> {
    run_checked(
        Command::new("cargo")
            .current_dir(repo_root)
            .args(["build", "--bins"]),
        "cargo build --bins",
    )?;
    run_checked(
        Command::new("cargo").current_dir(repo_root).args([
            "build",
            "-p",
            "sonar-indexer",
            "--lib",
        ]),
        "cargo build -p sonar-indexer --lib",
    )?;
    require_file(
        &repo_root.join("target/deploy/sonar_program.so"),
        "missing target/deploy/sonar_program.so; build the sonar BPF artifact first",
    )?;
    require_file(
        &repo_root.join("target/deploy/echo_callback.so"),
        "missing target/deploy/echo_callback.so; build the echo_callback BPF artifact first",
    )?;
    Ok(())
}

fn write_plugin_config(paths: &TestPaths, postgres_url: &str) -> Result<()> {
    let json = serde_json::json!({
        "libpath": paths.plugin_library(),
        "database_url": postgres_url,
        "log_level": "info",
        "max_connections": 4,
        "batch_size": 1,
    });
    fs::write(&paths.plugin_config, serde_json::to_vec_pretty(&json)?)
        .context("write geyser plugin config")
}

fn write_runtime_config(
    paths: &TestPaths,
    postgres_url: &str,
    redis_url: &str,
    ports: &PortLayout,
) -> Result<()> {
    let config = format!(
        r#"[network]
rpc_url = "{rpc_url}"
ws_url = "{ws_url}"
chain_id = "localnet"

[strategy]
min_profit_floor_usd = 0.01
gas_buffer_multiplier = 1.2
max_gas_price_gwei = 1.0

[rpc]
helius_api_key = "dummy"
helius_rpc_url = "{rpc_url}"

[indexer]
geyser_plugin_path = "{plugin_path}"
database_url = "{postgres_url}"
concurrency = 2
http_port = {indexer_http_port}

[coordinator]
redis_url = "{redis_url}"
callback_timeout_seconds = {callback_timeout_seconds}
max_concurrent_jobs = 4
indexer_url = "{indexer_url}"

[prover]
sp1_proving_key_path = "/tmp/sp1.key"
groth16_params_path = "/tmp/groth16.params"
mock_prover = true

[observability]
log_level = "info"
metrics_port = 9090
"#,
        rpc_url = ports.rpc_url(),
        ws_url = ports.ws_url(),
        plugin_path = paths.plugin_library().display(),
        postgres_url = postgres_url,
        indexer_http_port = ports.indexer_http_port,
        redis_url = redis_url,
        callback_timeout_seconds = callback_timeout().as_secs(),
        indexer_url = ports.indexer_url(),
    );
    fs::write(&paths.runtime_config, config).context("write runtime config")
}

fn write_historical_avg_verifier_registry_fixture(paths: &TestPaths) -> Result<()> {
    let (verifier_registry, bump) = historical_avg_verifier_registry_pda();
    let demo_fixture_path = paths
        .repo_root
        .join("program/tests/fixtures/demo_verifier_registry.json");
    let demo_fixture = fs::read_to_string(&demo_fixture_path)
        .with_context(|| format!("read {}", demo_fixture_path.display()))?;
    let demo_json: serde_json::Value =
        serde_json::from_str(&demo_fixture).context("parse demo verifier fixture json")?;
    let demo_data = demo_json["account"]["data"][0]
        .as_str()
        .context("read demo verifier fixture data")?;
    let decoded = BASE64_STANDARD
        .decode(demo_data)
        .context("decode demo verifier fixture base64")?;
    let mut slice = decoded.as_slice();
    let mut account = sonar_program::VerifierRegistry::try_deserialize(&mut slice)
        .context("deserialize demo verifier registry fixture")?;
    account.computation_id = HISTORICAL_AVG_COMPUTATION_ID;
    account.bump = bump;

    let mut data = Vec::new();
    account
        .try_serialize(&mut data)
        .context("serialize historical verifier registry")?;

    let fixture = serde_json::json!({
        "pubkey": verifier_registry.to_string(),
        "account": {
            "lamports": 10_000_000u64,
            "data": [BASE64_STANDARD.encode(&data), "base64"],
            "owner": sonar_program::id().to_string(),
            "executable": false,
            "rentEpoch": u64::MAX,
            "space": data.len(),
        }
    });

    fs::write(
        &paths.historical_avg_verifier_registry_fixture,
        serde_json::to_vec_pretty(&fixture).context("encode verifier registry fixture json")?,
    )
    .context("write historical verifier registry fixture")
}

fn start_validator(paths: &TestPaths, ports: &PortLayout) -> Result<ChildGuard> {
    let log = File::create(paths.log_path("validator")).context("create validator log")?;
    let (historical_avg_verifier_registry, _) = historical_avg_verifier_registry_pda();
    let mut command = Command::new("solana-test-validator");
    command
        .current_dir(&paths.repo_root)
        .arg("--reset")
        .arg("--quiet")
        .arg("--rpc-port")
        .arg(ports.rpc_port.to_string())
        .arg("--faucet-port")
        .arg(ports.faucet_port.to_string())
        .arg("--dynamic-port-range")
        .arg(format!(
            "{}-{}",
            ports.dynamic_port_start, ports.dynamic_port_end
        ))
        .arg("--bind-address")
        .arg("127.0.0.1")
        .arg("--ledger")
        .arg(&paths.ledger_dir)
        .arg("--geyser-plugin-config")
        .arg(&paths.plugin_config)
        .arg("--account")
        .arg(historical_avg_verifier_registry.to_string())
        .arg(&paths.historical_avg_verifier_registry_fixture)
        .arg("--bpf-program")
        .arg(sonar_program::id().to_string())
        .arg(paths.sonar_program_so())
        .arg("--bpf-program")
        .arg(echo_callback_program_id().to_string())
        .arg(paths.echo_callback_so())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    ChildGuard::spawn("validator", command)
}

fn echo_callback_program_id() -> Pubkey {
    Pubkey::from_str("3RBU9G6Mws9nS8bQPg2cVRbS2v7CgsjAvv2MwmTcmbyA")
        .expect("valid echo callback program id")
}

fn historical_avg_verifier_registry_pda() -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"verifier", &HISTORICAL_AVG_COMPUTATION_ID],
        &sonar_program::id(),
    )
}

fn start_indexer(paths: &TestPaths) -> Result<ChildGuard> {
    let log = File::create(paths.log_path("indexer")).context("create indexer log")?;
    let mut command = Command::new(paths.target_dir.join("debug/sonar-indexer"));
    command
        .current_dir(&paths.repo_root)
        .env("SONAR_CONFIG", &paths.runtime_config)
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    ChildGuard::spawn("indexer", command)
}

fn start_prover(paths: &TestPaths) -> Result<ChildGuard> {
    let log = File::create(paths.log_path("prover")).context("create prover log")?;
    let mut command = Command::new(paths.target_dir.join("debug/sonar-prover"));
    command
        .current_dir(&paths.repo_root)
        .env("SONAR_CONFIG", &paths.runtime_config)
        .env("SP1_PROVER", "mock")
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    ChildGuard::spawn("prover", command)
}

fn start_coordinator(paths: &TestPaths) -> Result<ChildGuard> {
    let log = File::create(paths.log_path("coordinator")).context("create coordinator log")?;
    let mut command = Command::new(paths.target_dir.join("debug/sonar-coordinator"));
    command
        .current_dir(&paths.repo_root)
        .env("SONAR_CONFIG_PATH", &paths.runtime_config)
        .env("SONAR_COORDINATOR_KEYPAIR_PATH", &paths.coordinator_keypair)
        .env("RUST_LOG", "sonar_coordinator=debug,info")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    ChildGuard::spawn("coordinator", command)
}

async fn wait_for_validator(
    validator: &mut ChildGuard,
    rpc_url: &str,
    validator_log: &Path,
) -> Result<()> {
    let deadline = Instant::now() + VALIDATOR_READY_TIMEOUT;
    loop {
        if let Err(error) = validator.check_running() {
            let log_output = read_log_output(validator_log);
            return Err(error).context(format!(
                "validator exited before becoming ready\n{}",
                log_output
            ));
        }

        let client =
            RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
        match client.get_latest_blockhash() {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await
            },
            Err(error) => {
                let log_output = read_log_output(validator_log);
                return Err(error).context(format!(
                    "validator did not become ready in time\n{}",
                    log_output
                ));
            },
        }
    }
}

async fn wait_for_indexer(
    indexer: &mut ChildGuard,
    indexer_url: &str,
    indexer_log: &Path,
) -> Result<()> {
    let deadline = Instant::now() + HTTP_READY_TIMEOUT;
    let probe = format!(
        "{indexer_url}/account_history/11111111111111111111111111111111?from_slot=0&to_slot=0"
    );
    loop {
        if let Err(error) = indexer.check_running() {
            let log_output = read_log_output(indexer_log);
            return Err(error).context(format!(
                "indexer exited before becoming ready\n{}",
                log_output
            ));
        }

        match reqwest::get(&probe).await {
            Ok(response) if response.status() == StatusCode::OK => return Ok(()),
            _ if Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(500)).await,
            Ok(response) => {
                let log_output = read_log_output(indexer_log);
                bail!(
                    "indexer probe failed with status {}\n{}",
                    response.status(),
                    log_output
                );
            },
            Err(error) => {
                let log_output = read_log_output(indexer_log);
                return Err(error).context(format!(
                    "indexer did not become ready in time\n{}",
                    log_output
                ));
            },
        }
    }
}

async fn seed_account_history(
    rpc: &RpcClient,
    payer: &Keypair,
    observed: &Keypair,
) -> Result<Vec<BalancePoint>> {
    let mut balances = Vec::new();

    let initial_lamports = 200_000_000;
    send_transaction(
        rpc,
        payer,
        &[payer, observed],
        &[system_instruction::create_account(
            &payer.pubkey(),
            &observed.pubkey(),
            initial_lamports,
            0,
            &system_program::id(),
        )],
    )?;
    let mut last_slot = rpc.get_slot()?;
    balances.push(BalancePoint {
        slot: last_slot,
        lamports: rpc.get_balance(&observed.pubkey())?,
    });

    wait_for_next_slot(rpc, last_slot).await?;
    send_transaction(
        rpc,
        payer,
        &[payer],
        &[system_instruction::transfer(
            &payer.pubkey(),
            &observed.pubkey(),
            80_000_000,
        )],
    )?;
    last_slot = rpc.get_slot()?;
    balances.push(BalancePoint {
        slot: last_slot,
        lamports: rpc.get_balance(&observed.pubkey())?,
    });

    wait_for_next_slot(rpc, last_slot).await?;
    send_transaction(
        rpc,
        payer,
        &[payer, observed],
        &[system_instruction::transfer(
            &observed.pubkey(),
            &payer.pubkey(),
            50_000_000,
        )],
    )?;
    last_slot = rpc.get_slot()?;
    balances.push(BalancePoint {
        slot: last_slot,
        lamports: rpc.get_balance(&observed.pubkey())?,
    });

    wait_for_next_slot(rpc, last_slot).await?;
    send_transaction(
        rpc,
        payer,
        &[payer],
        &[system_instruction::transfer(
            &payer.pubkey(),
            &observed.pubkey(),
            170_000_000,
        )],
    )?;
    balances.push(BalancePoint {
        slot: rpc.get_slot()?,
        lamports: rpc.get_balance(&observed.pubkey())?,
    });

    Ok(balances)
}

async fn wait_for_indexed_balances(
    indexer_url: &str,
    observed_pubkey: &Pubkey,
    from_slot: u64,
    to_slot: u64,
    expected: &[u64],
    validator_log: &Path,
    indexer_log: &Path,
) -> Result<()> {
    let deadline = Instant::now() + SEED_WAIT_TIMEOUT;
    let pubkey = bs58::encode(observed_pubkey).into_string();
    let url =
        format!("{indexer_url}/account_history/{pubkey}?from_slot={from_slot}&to_slot={to_slot}");
    #[allow(unused_assignments)]
    let mut last_status = None;
    #[allow(unused_assignments)]
    let mut last_body = None;

    loop {
        match reqwest::get(&url).await {
            Ok(response) if response.status() == StatusCode::OK => {
                let body = response.text().await?;
                last_status = Some(StatusCode::OK);
                last_body = Some(body.clone());
                let balances = serde_json::from_str::<Vec<u64>>(&body)
                    .context("deserialize indexed balances")?;
                if balances == expected {
                    return Ok(());
                }
            },
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.ok();
                last_status = Some(status);
                last_body = body;
            },
            Err(error) => {
                last_status = None;
                last_body = Some(error.to_string());
            },
        }

        if Instant::now() >= deadline {
            let validator_output = read_log_output(validator_log);
            let indexer_output = read_log_output(indexer_log);
            bail!(
                "indexer never returned expected balances for {url}; expected {expected:?}, last_status={last_status:?}, last_body={last_body:?}\n{validator_output}\n{indexer_output}"
            );
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn submit_historical_average_request(
    rpc: &RpcClient,
    payer: &Keypair,
    observed_account: &Pubkey,
    from_slot: u64,
    to_slot: u64,
    request_id: [u8; 32],
) -> Result<Pubkey> {
    let (request_metadata, _) =
        Pubkey::find_program_address(&[b"request", &request_id], &sonar_program::id());
    let (result_account, _) =
        Pubkey::find_program_address(&[b"result", &request_id], &sonar_program::id());
    let current_slot = rpc.get_slot()?;
    let mut raw_inputs = Vec::with_capacity(48);
    raw_inputs.extend_from_slice(&observed_account.to_bytes());
    raw_inputs.extend_from_slice(&from_slot.to_le_bytes());
    raw_inputs.extend_from_slice(&to_slot.to_le_bytes());

    let params = RequestParams {
        request_id,
        computation_id: HISTORICAL_AVG_COMPUTATION_ID,
        inputs: raw_inputs,
        deadline: current_slot + 500,
        fee: REQUEST_FEE_LAMPORTS,
    };

    let instruction = Instruction {
        program_id: sonar_program::id(),
        accounts: sonar_accounts::Request {
            payer: payer.pubkey(),
            callback_program: echo_callback_program_id(),
            request_metadata,
            result_account,
            system_program: system_program::id(),
        }
        .to_account_metas(None),
        data: sonar_instruction::Request { params }.data(),
    };

    send_transaction(rpc, payer, &[payer], &[instruction])?;
    Ok(result_account)
}

async fn wait_for_coordinator_ready(
    coordinator: &mut ChildGuard,
    coordinator_log: &Path,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Err(error) = coordinator.check_running() {
            let log_output = read_log_output(coordinator_log);
            return Err(error).context(format!(
                "coordinator exited before becoming ready\n{log_output}"
            ));
        }

        if let Ok(contents) = fs::read_to_string(coordinator_log) {
            if contents.contains("Listener started") {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            let log_output = read_log_output(coordinator_log);
            bail!("coordinator did not become ready (no 'Listener started' in log)\n{log_output}");
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_result_account(
    rpc: &RpcClient,
    result_account: Pubkey,
    expected_avg: u64,
    validator_log: &Path,
    coordinator_log: &Path,
    prover_log: &Path,
) -> Result<ResultAccount> {
    let timeout = callback_timeout();
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(account) = rpc.get_account(&result_account) {
            let mut data = account.data.as_slice();
            let state =
                ResultAccount::try_deserialize(&mut data).context("deserialize result account")?;
            if state.is_set {
                let value = decode_u64(&state.result)?;
                if value == expected_avg {
                    return Ok(state);
                }
            }
        }

        if Instant::now() >= deadline {
            let validator_output = read_log_output(validator_log);
            let coordinator_output = read_log_output(coordinator_log);
            let prover_output = read_log_output(prover_log);
            bail!(
                "timed out waiting for historical-average result account after {}s (expected avg={expected_avg})\n{validator_output}\n{coordinator_output}\n{prover_output}",
                timeout.as_secs()
            );
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn read_anchor_account<T: AccountDeserialize>(rpc: &RpcClient, pubkey: Pubkey) -> Result<T> {
    let account = rpc
        .get_account(&pubkey)
        .with_context(|| format!("fetch account {pubkey}"))?;
    let mut data = account.data.as_slice();
    T::try_deserialize(&mut data).with_context(|| format!("deserialize account {pubkey}"))
}

fn send_transaction(
    rpc: &RpcClient,
    payer: &Keypair,
    signers: &[&Keypair],
    instructions: &[Instruction],
) -> Result<()> {
    let blockhash = rpc.get_latest_blockhash().context("get latest blockhash")?;
    let transaction =
        Transaction::new_signed_with_payer(instructions, Some(&payer.pubkey()), signers, blockhash);
    rpc.send_and_confirm_transaction(&transaction)
        .context("send and confirm transaction")?;
    Ok(())
}

fn airdrop(rpc: &RpcClient, recipient: &Pubkey, lamports: u64) -> Result<()> {
    let starting_balance = rpc
        .get_balance(recipient)
        .with_context(|| format!("read starting balance for {recipient}"))?;
    let signature = rpc
        .request_airdrop(recipient, lamports)
        .with_context(|| format!("airdrop {lamports} lamports to {recipient}"))?;
    rpc.confirm_transaction(&signature)
        .context("confirm airdrop transaction")?;

    let expected_balance = starting_balance.saturating_add(lamports);
    wait_for_balance(rpc, recipient, expected_balance)
        .with_context(|| format!("wait for airdrop funds to land for {recipient}"))?;
    Ok(())
}

fn wait_for_balance(rpc: &RpcClient, recipient: &Pubkey, minimum_balance: u64) -> Result<()> {
    let deadline = Instant::now() + AIRDROP_WAIT_TIMEOUT;
    loop {
        let balance = rpc
            .get_balance(recipient)
            .with_context(|| format!("get balance for {recipient}"))?;
        if balance >= minimum_balance {
            return Ok(());
        }

        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for {recipient} to reach balance {minimum_balance}; current balance {balance}"
            );
        }

        std::thread::sleep(Duration::from_millis(250));
    }
}

async fn wait_for_next_slot(rpc: &RpcClient, current_slot: u64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let slot = rpc.get_slot().context("get current slot")?;
        if slot > current_slot {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for validator slot to advance past {current_slot}");
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

fn run_checked(command: &mut Command, label: &str) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("failed to execute {label}"))?;
    if !output.status.success() {
        bail!(
            "{label} failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}

fn find_available_port(reserved_ports: &[u16], excluded_ranges: &[(u16, u16)]) -> Result<u16> {
    for port in PORT_SCAN_START..=PORT_SCAN_END {
        if reserved_ports.contains(&port) {
            continue;
        }

        if excluded_ranges
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&port))
        {
            continue;
        }

        if port_is_available(port) {
            return Ok(port);
        }
    }

    bail!("failed to find an available TCP port")
}

/// Like [`find_available_port`] but also verifies that `port + 1` (the Solana
/// WebSocket port, which is always `rpc_port + 1`) is free as well.
fn find_available_rpc_port(excluded_ranges: &[(u16, u16)]) -> Result<u16> {
    for port in PORT_SCAN_START..PORT_SCAN_END {
        let ws_port = port + 1;

        if excluded_ranges.iter().any(|(start, end)| {
            (*start..=*end).contains(&port) || (*start..=*end).contains(&ws_port)
        }) {
            continue;
        }

        if port_is_available(port) && port_is_available(ws_port) {
            return Ok(port);
        }
    }

    bail!("failed to find an available RPC+WS port pair")
}

fn find_available_port_range(port_count: u16) -> Result<(u16, u16)> {
    let max_start = PORT_SCAN_END
        .checked_sub(port_count.saturating_sub(1))
        .context("invalid port range size")?;

    for start in PORT_SCAN_START..=max_start {
        let end = start + port_count - 1;
        if (start..=end).all(port_is_available) {
            return Ok((start, end));
        }
    }

    bail!("failed to find an available validator port range")
}

fn port_is_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn require_file(path: &Path, message: &str) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }

    bail!("{message}: {}", path.display())
}

fn read_log_output(path: &Path) -> String {
    match fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => {
            format!("validator log at {} is empty", path.display())
        },
        Ok(contents) => format!("validator log ({}):\n{}", path.display(), contents),
        Err(error) => format!(
            "failed to read validator log at {}: {error}",
            path.display()
        ),
    }
}

fn decode_u64(bytes: &[u8]) -> Result<u64> {
    let array: [u8; 8] = bytes
        .try_into()
        .map_err(|_| anyhow!("expected exactly 8 result bytes, got {}", bytes.len()))?;
    Ok(u64::from_le_bytes(array))
}
