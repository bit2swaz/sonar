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

use crate::{prove, registry::resolve_computation};

pub const DEFAULT_JOB_QUEUE: &str = "sonar:jobs";
pub const DEFAULT_RESPONSE_QUEUE: &str = "sonar:responses";

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
        debug!(
            request_id = ?job.request_id,
            computation = computation.name,
            elf_path = computation.elf_path,
            "processing prover job"
        );

        let (proof, result, public_inputs) = prove(&job.computation_id, &job.inputs)?;
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
