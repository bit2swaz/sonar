use std::{sync::Arc, time::Duration};

use anyhow::Context;
use async_trait::async_trait;
use redis::Client;
use sonar_common::{
    config::Config,
    types::{ProverJob, ProverResponse},
};
use tokio::{
    sync::{watch, Semaphore},
    task::JoinSet,
};
use tracing::{debug, error, info, warn};

use crate::{prove, registry::resolve_computation, sp1_wrapper::compute_historical_avg_result};

pub const DEFAULT_JOB_QUEUE: &str = "sonar:jobs";
pub const DEFAULT_RESPONSE_QUEUE: &str = "sonar:responses";

const MOCK_HISTORICAL_AVG_PROOF: [u8; 256] = [
    45, 206, 255, 166, 152, 55, 128, 138, 79, 217, 145, 164, 25, 74, 120, 234,
    234, 217, 68, 149, 162, 44, 133, 120, 184, 205, 12, 44, 175, 98, 168, 172,
    28, 75, 118, 99, 15, 130, 53, 222, 36, 99, 235, 81, 5, 165, 98, 197,
    197, 182, 144, 40, 212, 105, 169, 142, 72, 96, 177, 156, 174, 43, 59, 243,
    40, 57, 233, 205, 180, 46, 35, 111, 215, 5, 23, 93, 12, 71, 118, 225,
    7, 46, 247, 147, 47, 130, 106, 189, 184, 80, 146, 103, 141, 52, 242, 25,
    0, 203, 124, 176, 110, 34, 151, 212, 66, 180, 238, 151, 236, 189, 133, 209,
    17, 137, 205, 183, 168, 196, 92, 159, 75, 174, 81, 168, 18, 86, 176, 56,
    16, 26, 210, 20, 18, 81, 122, 142, 104, 62, 251, 169, 98, 141, 21, 253,
    50, 130, 182, 15, 33, 109, 228, 31, 79, 183, 88, 147, 174, 108, 4, 22,
    14, 129, 168, 6, 80, 246, 254, 100, 218, 131, 94, 49, 247, 211, 3, 245,
    22, 200, 177, 91, 60, 144, 147, 174, 90, 17, 19, 189, 62, 147, 152, 18,
    41, 139, 183, 208, 246, 198, 118, 127, 89, 160, 9, 27, 61, 26, 123, 180,
    221, 108, 17, 166, 47, 115, 82, 48, 132, 139, 253, 65, 152, 92, 209, 53,
    37, 25, 83, 61, 252, 42, 181, 243, 16, 21, 2, 199, 123, 96, 218, 151,
    253, 86, 69, 181, 202, 109, 64, 129, 124, 254, 192, 25, 177, 199, 26, 50,
];

const MOCK_HISTORICAL_AVG_PUBLIC_INPUTS: [[u8; 32]; 9] = [
    [34, 238, 251, 182, 234, 248, 214, 189, 46, 67, 42, 25, 71, 58, 145, 58, 61, 28, 116, 110, 60, 17, 82, 149, 178, 187, 160, 211, 37, 226, 174, 231],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 51, 152, 17, 147],
    [4, 247, 199, 87, 230, 85, 103, 90, 28, 183, 95, 100, 200, 46, 3, 158, 247, 196, 173, 146, 207, 167, 108, 33, 199, 18, 13, 204, 198, 101, 223, 186],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 49, 65, 41],
    [7, 130, 55, 65, 197, 232, 175, 217, 44, 151, 149, 225, 75, 86, 158, 105, 43, 229, 65, 87, 51, 150, 168, 243, 176, 175, 11, 203, 180, 149, 72, 103],
    [46, 93, 177, 62, 42, 66, 223, 153, 51, 193, 146, 49, 154, 41, 69, 198, 224, 13, 87, 80, 222, 171, 37, 141, 0, 1, 50, 172, 18, 28, 213, 213],
    [40, 141, 45, 3, 180, 200, 250, 112, 108, 94, 35, 143, 82, 63, 125, 9, 147, 37, 191, 75, 62, 221, 138, 20, 166, 151, 219, 237, 254, 58, 230, 189],
    [33, 100, 143, 241, 11, 251, 73, 141, 229, 57, 129, 168, 83, 23, 235, 147, 138, 225, 177, 250, 13, 97, 226, 162, 6, 232, 52, 95, 128, 84, 90, 202],
    [25, 178, 1, 208, 219, 169, 222, 123, 113, 202, 165, 77, 183, 98, 103, 237, 187, 93, 178, 95, 169, 156, 38, 100, 125, 218, 104, 94, 104, 119, 13, 21],
];

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub max_concurrent_jobs: usize,
    pub poll_timeout: Duration,
}

impl ServiceConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            max_concurrent_jobs: config.coordinator.max_concurrent_jobs.max(1),
            poll_timeout: Duration::from_secs(1),
        }
    }
}

#[async_trait]
pub trait ProverQueue: Send + Sync {
    async fn pop_job(&self, timeout: Duration) -> anyhow::Result<Option<String>>;
    async fn push_response(&self, payload: String) -> anyhow::Result<()>;
}

#[async_trait]
pub trait JobProcessor: Send + Sync {
    async fn process(&self, job: ProverJob) -> anyhow::Result<ProverResponse>;
}

#[derive(Debug, Clone)]
pub struct RedisQueue {
    client: Client,
    job_queue: String,
    response_queue: String,
}

impl RedisQueue {
    pub fn new(redis_url: &str) -> anyhow::Result<Self> {
        Self::with_queue_names(redis_url, DEFAULT_JOB_QUEUE, DEFAULT_RESPONSE_QUEUE)
    }

    pub fn with_queue_names(
        redis_url: &str,
        job_queue: impl Into<String>,
        response_queue: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let client = Client::open(redis_url)
            .with_context(|| format!("failed to create Redis client for {redis_url}"))?;
        Ok(Self {
            client,
            job_queue: job_queue.into(),
            response_queue: response_queue.into(),
        })
    }
}

#[async_trait]
impl ProverQueue for RedisQueue {
    async fn pop_job(&self, timeout: Duration) -> anyhow::Result<Option<String>> {
        let mut connection = self
            .client
            .get_async_connection()
            .await
            .context("failed to open async Redis connection for job pop")?;
        let timeout_secs = timeout.as_secs().max(1);
        let payload = redis::cmd("BLPOP")
            .arg(&self.job_queue)
            .arg(timeout_secs)
            .query_async::<_, Option<(String, String)>>(&mut connection)
            .await
            .with_context(|| format!("failed to read from Redis queue {}", self.job_queue))?;

        Ok(payload.map(|(_, job)| job))
    }

    async fn push_response(&self, payload: String) -> anyhow::Result<()> {
        let mut connection = self
            .client
            .get_async_connection()
            .await
            .context("failed to open async Redis connection for response push")?;
        redis::cmd("RPUSH")
            .arg(&self.response_queue)
            .arg(payload)
            .query_async::<_, i64>(&mut connection)
            .await
            .with_context(|| format!("failed to push to Redis queue {}", self.response_queue))?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Sp1JobProcessor;

#[async_trait]
impl JobProcessor for Sp1JobProcessor {
    async fn process(&self, job: ProverJob) -> anyhow::Result<ProverResponse> {
        let computation = resolve_computation(&job.computation_id)?;
        let mock_historical_avg = std::env::var("SP1_PROVER")
            .map(|value| value.eq_ignore_ascii_case("mock"))
            .unwrap_or(false)
            && computation.name == "historical_avg";

        debug!(
            request_id = ?job.request_id,
            computation = computation.name,
            elf_path = computation.elf_path,
            "processing prover job"
        );

        if mock_historical_avg {
            let balances: Vec<u64> = bincode::deserialize(&job.inputs)
                .context("failed to deserialize mock historical_avg inputs")?;
            let result = compute_historical_avg_result(&balances).to_le_bytes().to_vec();

            return Ok(ProverResponse {
                request_id: job.request_id,
                result,
                gas_used: MOCK_HISTORICAL_AVG_PROOF.len() as u64,
                proof: MOCK_HISTORICAL_AVG_PROOF.to_vec(),
                public_inputs: MOCK_HISTORICAL_AVG_PUBLIC_INPUTS
                    .iter()
                    .map(|input| input.to_vec())
                    .collect(),
            });
        }

        // `prove()` calls SP1's internal `block_on`, which panics if invoked
        // inside a tokio async context.  Offload it to a dedicated blocking
        // thread via `spawn_blocking` so the tokio reactor is not blocked.
        let computation_id = job.computation_id;
        let inputs = job.inputs.clone();
        let (proof, result, public_inputs) =
            tokio::task::spawn_blocking(move || prove(&computation_id, &inputs))
                .await
                .context("prover blocking task panicked")??;

        Ok(ProverResponse {
            request_id: job.request_id,
            result,
            gas_used: proof.len() as u64,
            proof,
            public_inputs: vec![public_inputs],
        })
    }
}

pub async fn run_redis_service(
    config: &Config,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let queue = Arc::new(RedisQueue::new(&config.coordinator.redis_url)?);
    let processor = Arc::new(Sp1JobProcessor);
    run_service(
        queue,
        processor,
        ServiceConfig::from_config(config),
        shutdown,
    )
    .await
}

pub async fn run_service<Q, P>(
    queue: Arc<Q>,
    processor: Arc<P>,
    config: ServiceConfig,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()>
where
    Q: ProverQueue + 'static,
    P: JobProcessor + 'static,
{
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_jobs.max(1)));
    let mut shutdown = shutdown;
    let mut tasks = JoinSet::new();

    info!(
        max_concurrent_jobs = config.max_concurrent_jobs,
        job_queue = DEFAULT_JOB_QUEUE,
        response_queue = DEFAULT_RESPONSE_QUEUE,
        "starting prover service"
    );

    loop {
        drain_finished_tasks(&mut tasks);

        if *shutdown.borrow() {
            info!("shutdown requested; draining in-flight prover tasks");
            break;
        }

        let Some(payload) = queue.pop_job(config.poll_timeout).await? else {
            continue;
        };

        let job = match serde_json::from_str::<ProverJob>(&payload) {
            Ok(job) => job,
            Err(error) => {
                warn!(%error, payload, "dropping malformed prover job payload");
                continue;
            },
        };

        let permit = tokio::select! {
            acquire = semaphore.clone().acquire_owned() => acquire.context("prover service semaphore closed")?,
            changed = shutdown.changed() => {
                changed.context("failed waiting for shutdown signal")?;
                info!(request_id = ?job.request_id, "shutdown requested before scheduling job");
                break;
            }
        };

        let queue = Arc::clone(&queue);
        let processor = Arc::clone(&processor);
        tasks.spawn(async move {
            let _permit = permit;
            process_job(queue, processor, job).await;
        });
    }

    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            error!(%error, "prover worker task failed to join cleanly");
        }
    }

    info!("prover service stopped cleanly");
    Ok(())
}

async fn process_job<Q, P>(queue: Arc<Q>, processor: Arc<P>, job: ProverJob)
where
    Q: ProverQueue + 'static,
    P: JobProcessor + 'static,
{
    let request_id = job.request_id;
    match processor.process(job).await {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(payload) => {
                if let Err(error) = queue.push_response(payload).await {
                    error!(%error, request_id = ?request_id, "failed to publish prover response");
                } else {
                    info!(request_id = ?request_id, "published prover response");
                }
            },
            Err(error) => {
                error!(%error, request_id = ?request_id, "failed to serialize prover response");
            },
        },
        Err(error) => {
            error!(%error, request_id = ?request_id, "prover job failed");
        },
    }
}

fn drain_finished_tasks(tasks: &mut JoinSet<()>) {
    while let Some(result) = tasks.try_join_next() {
        if let Err(error) = result {
            error!(%error, "prover worker task failed to join cleanly");
        }
    }
}
