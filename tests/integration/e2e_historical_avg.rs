#![allow(deprecated)]

use std::{
    fs::{self, File},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anyhow::{anyhow, bail, Context, Result};
use historical_avg_client::{
    accounts as historical_avg_accounts, instruction as historical_avg_instruction, CallbackState,
    HistoricalAvgRequestParams,
};
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
use sonar_program::{RequestMetadata, RequestStatus, ResultAccount};
use tempfile::TempDir;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    GenericImage, ImageExt,
};

const VALIDATOR_READY_TIMEOUT: Duration = Duration::from_secs(45);
const HTTP_READY_TIMEOUT: Duration = Duration::from_secs(45);
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);
const SEED_WAIT_TIMEOUT: Duration = Duration::from_secs(45);
const REQUEST_FEE_LAMPORTS: u64 = 2_000_000;

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
#[ignore = "requires docker, anchor, solana-test-validator, and local BPF program artifacts"]
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

    let _validator = start_validator(&paths, &ports)?;
    wait_for_validator(&ports.rpc_url()).await?;

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
    wait_for_indexer(&ports.indexer_url()).await?;

    let observed = Keypair::new();
    let balances = seed_account_history(&rpc, &client, &observed).await?;
    let expected_balances: Vec<u64> = balances.iter().map(|point| point.lamports).collect();
    let expected_avg = expected_balances.iter().sum::<u64>() / expected_balances.len() as u64;
    wait_for_indexed_balances(
        &ports.indexer_url(),
        &observed.pubkey(),
        balances.first().expect("seeded slots").slot,
        balances.last().expect("seeded slots").slot,
        &expected_balances,
    )
    .await?;

    let mut prover = start_prover(&paths)?;
    let mut coordinator_worker = start_coordinator(&paths)?;
    indexer.check_running()?;
    prover.check_running()?;
    coordinator_worker.check_running()?;

    let request_id = Keypair::new().pubkey().to_bytes();
    let callback_state = submit_historical_average_request(
        &rpc,
        &client,
        &observed.pubkey(),
        balances.first().expect("seeded slots").slot,
        balances.last().expect("seeded slots").slot,
        request_id,
    )?;

    let callback_state = wait_for_callback_state(&rpc, callback_state, expected_avg).await?;
    assert!(callback_state.is_set, "client callback state should be set");
    assert_eq!(decode_u64(&callback_state.result)?, expected_avg);

    let (request_metadata_pda, _) =
        Pubkey::find_program_address(&[b"request", &request_id], &sonar_program::id());
    let (result_account_pda, _) =
        Pubkey::find_program_address(&[b"result", &request_id], &sonar_program::id());

    let request_metadata = read_anchor_account::<RequestMetadata>(&rpc, request_metadata_pda)?;
    assert!(matches!(request_metadata.status, RequestStatus::Completed));

    let result_account = read_anchor_account::<ResultAccount>(&rpc, result_account_pda)?;
    assert!(
        result_account.is_set,
        "sonar result account should be written"
    );
    assert_eq!(decode_u64(&result_account.result)?, expected_avg);

    Ok(())
}

struct TestPaths {
    repo_root: PathBuf,
    log_dir: PathBuf,
    ledger_dir: PathBuf,
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

    fn historical_avg_client_so(&self) -> PathBuf {
        self.target_dir.join("deploy/historical_avg_client.so")
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
        let rpc_port = free_port()?;
        let faucet_port = free_port()?;
        let dynamic_port_start = free_port()?;
        let dynamic_port_end = dynamic_port_start + 10;
        let indexer_http_port = free_port()?;

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
    run_checked(
        Command::new("anchor").current_dir(repo_root).arg("build"),
        "anchor build",
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
rpc_url = \"{rpc_url}\"
ws_url = \"{ws_url}\"
chain_id = \"localnet\"

[strategy]
min_profit_floor_usd = 0.01
gas_buffer_multiplier = 1.2
max_gas_price_gwei = 1.0

[rpc]
helius_api_key = \"dummy\"
helius_rpc_url = \"{rpc_url}\"

[indexer]
geyser_plugin_path = \"{plugin_path}\"
database_url = \"{postgres_url}\"
concurrency = 2
http_port = {indexer_http_port}

[coordinator]
redis_url = \"{redis_url}\"
callback_timeout_seconds = 30
max_concurrent_jobs = 4
indexer_url = \"{indexer_url}\"

[prover]
sp1_proving_key_path = \"/tmp/sp1.key\"
groth16_params_path = \"/tmp/groth16.params\"
mock_prover = true

[observability]
log_level = \"info\"
metrics_port = 9090
"#,
        rpc_url = ports.rpc_url(),
        ws_url = ports.ws_url(),
        plugin_path = paths.plugin_library().display(),
        postgres_url = postgres_url,
        indexer_http_port = ports.indexer_http_port,
        redis_url = redis_url,
        indexer_url = ports.indexer_url(),
    );
    fs::write(&paths.runtime_config, config).context("write runtime config")
}

fn start_validator(paths: &TestPaths, ports: &PortLayout) -> Result<ChildGuard> {
    let log = File::create(paths.log_path("validator")).context("create validator log")?;
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
        .arg("--bpf-program")
        .arg(sonar_program::id().to_string())
        .arg(paths.sonar_program_so())
        .arg("--bpf-program")
        .arg(historical_avg_client::id().to_string())
        .arg(paths.historical_avg_client_so())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    ChildGuard::spawn("validator", command)
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
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    ChildGuard::spawn("coordinator", command)
}

async fn wait_for_validator(rpc_url: &str) -> Result<()> {
    let deadline = Instant::now() + VALIDATOR_READY_TIMEOUT;
    loop {
        let client =
            RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
        match client.get_latest_blockhash() {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await
            },
            Err(error) => return Err(error).context("validator did not become ready in time"),
        }
    }
}

async fn wait_for_indexer(indexer_url: &str) -> Result<()> {
    let deadline = Instant::now() + HTTP_READY_TIMEOUT;
    let probe = format!(
        "{indexer_url}/account_history/11111111111111111111111111111111?from_slot=0&to_slot=0"
    );
    loop {
        match reqwest::get(&probe).await {
            Ok(response) if response.status() == StatusCode::OK => return Ok(()),
            _ if Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(500)).await,
            Ok(response) => {
                bail!("indexer probe failed with status {}", response.status());
            },
            Err(error) => return Err(error).context("indexer did not become ready in time"),
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
        &[&payer, observed],
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
) -> Result<()> {
    let deadline = Instant::now() + SEED_WAIT_TIMEOUT;
    let pubkey = bs58::encode(observed_pubkey).into_string();
    let url =
        format!("{indexer_url}/account_history/{pubkey}?from_slot={from_slot}&to_slot={to_slot}");

    loop {
        match reqwest::get(&url).await {
            Ok(response) if response.status() == StatusCode::OK => {
                let balances = response.json::<Vec<u64>>().await?;
                if balances == expected {
                    return Ok(());
                }
            },
            _ => {},
        }

        if Instant::now() >= deadline {
            bail!("indexer never returned expected balances for {url}");
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
    let (callback_state, _) = Pubkey::find_program_address(
        &[b"client-result", &request_id],
        &historical_avg_client::id(),
    );
    let (request_metadata, _) =
        Pubkey::find_program_address(&[b"request", &request_id], &sonar_program::id());
    let (result_account, _) =
        Pubkey::find_program_address(&[b"result", &request_id], &sonar_program::id());
    let current_slot = rpc.get_slot()?;
    let params = HistoricalAvgRequestParams {
        request_id,
        account: observed_account.to_bytes(),
        from_slot,
        to_slot,
        fee: REQUEST_FEE_LAMPORTS,
        deadline: current_slot + 500,
    };

    let instruction = Instruction {
        program_id: historical_avg_client::id(),
        accounts: historical_avg_accounts::RequestHistoricalAvg {
            payer: payer.pubkey(),
            callback_program: historical_avg_client::id(),
            callback_state,
            request_metadata,
            result_account,
            sonar_program: sonar_program::id(),
            system_program: system_program::id(),
        }
        .to_account_metas(None),
        data: historical_avg_instruction::RequestHistoricalAvg { params }.data(),
    };

    send_transaction(rpc, payer, &[payer], &[instruction])?;
    Ok(callback_state)
}

async fn wait_for_callback_state(
    rpc: &RpcClient,
    callback_state: Pubkey,
    expected_avg: u64,
) -> Result<CallbackState> {
    let deadline = Instant::now() + CALLBACK_TIMEOUT;
    loop {
        if let Ok(account) = rpc.get_account(&callback_state) {
            let mut data = account.data.as_slice();
            let state =
                CallbackState::try_deserialize(&mut data).context("deserialize callback state")?;
            if state.is_set {
                let value = decode_u64(&state.result)?;
                if value == expected_avg {
                    return Ok(state);
                }
            }
        }

        if Instant::now() >= deadline {
            bail!("timed out waiting for historical-average callback state");
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
    let signature = rpc
        .request_airdrop(recipient, lamports)
        .with_context(|| format!("airdrop {lamports} lamports to {recipient}"))?;
    rpc.confirm_transaction(&signature)
        .context("confirm airdrop transaction")?;
    Ok(())
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

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("bind ephemeral port")?;
    let port = listener.local_addr().context("read local addr")?.port();
    drop(listener);
    Ok(port)
}

fn decode_u64(bytes: &[u8]) -> Result<u64> {
    let array: [u8; 8] = bytes
        .try_into()
        .map_err(|_| anyhow!("expected exactly 8 result bytes, got {}", bytes.len()))?;
    Ok(u64::from_le_bytes(array))
}
