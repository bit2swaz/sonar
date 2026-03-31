use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::anyhow;
use async_trait::async_trait;
use sonar_common::types::{ProverJob, ProverResponse, Pubkey};
use sonar_prover::service::{run_service, JobProcessor, ProverQueue, ServiceConfig};
use tokio::{
    sync::{watch, Mutex, Notify},
    time::{sleep, timeout},
};

#[derive(Default)]
struct MockQueue {
    jobs: Mutex<VecDeque<String>>,
    responses: Mutex<Vec<String>>,
    job_notify: Notify,
}

impl MockQueue {
    async fn enqueue_job(&self, job: ProverJob) {
        let payload = serde_json::to_string(&job).expect("job should serialize");
        self.jobs.lock().await.push_back(payload);
        self.job_notify.notify_waiters();
    }

    async fn response_count(&self) -> usize {
        self.responses.lock().await.len()
    }

    async fn responses(&self) -> Vec<String> {
        self.responses.lock().await.clone()
    }
}

#[async_trait]
impl ProverQueue for MockQueue {
    async fn pop_job(&self, timeout_duration: Duration) -> anyhow::Result<Option<String>> {
        loop {
            if let Some(job) = self.jobs.lock().await.pop_front() {
                return Ok(Some(job));
            }

            if timeout(timeout_duration, self.job_notify.notified())
                .await
                .is_err()
            {
                return Ok(None);
            }
        }
    }

    async fn push_response(&self, payload: String) -> anyhow::Result<()> {
        self.responses.lock().await.push(payload);
        Ok(())
    }
}

struct MockProcessor {
    delay: Duration,
    fail_request_id: Option<[u8; 32]>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    started: AtomicUsize,
}

impl MockProcessor {
    fn new(delay: Duration) -> Self {
        Self {
            delay,
            fail_request_id: None,
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
        }
    }

    fn failing(delay: Duration, fail_request_id: [u8; 32]) -> Self {
        Self {
            delay,
            fail_request_id: Some(fail_request_id),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
        }
    }

    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }

    fn record_max_active(&self, current: usize) {
        let mut observed = self.max_active.load(Ordering::SeqCst);
        while current > observed {
            match self.max_active.compare_exchange(
                observed,
                current,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(actual) => observed = actual,
            }
        }
    }
}

#[async_trait]
impl JobProcessor for MockProcessor {
    async fn process(&self, job: ProverJob) -> anyhow::Result<ProverResponse> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_max_active(active);

        sleep(self.delay).await;
        self.active.fetch_sub(1, Ordering::SeqCst);

        if self.fail_request_id == Some(job.request_id) {
            return Err(anyhow!("forced processor failure"));
        }

        Ok(ProverResponse {
            request_id: job.request_id,
            result: job.inputs.clone(),
            proof: vec![1, 2, 3, 4],
            public_inputs: vec![job.inputs.clone()],
            gas_used: 4,
        })
    }
}

fn sample_job(seed: u8) -> ProverJob {
    ProverJob {
        request_id: [seed; 32],
        computation_id: [seed.wrapping_add(1); 32],
        inputs: vec![seed, seed.wrapping_add(1)],
        deadline: 42,
        fee: 100,
        callback_program: Pubkey::new([seed.wrapping_add(2); 32]),
        result_account: Pubkey::new([seed.wrapping_add(3); 32]),
    }
}

async fn wait_for_response_count(queue: &MockQueue, expected: usize) {
    timeout(Duration::from_secs(2), async {
        loop {
            if queue.response_count().await >= expected {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expected responses to arrive in time");
}

#[tokio::test]
async fn test_run_service_processes_jobs_and_publishes_responses() {
    let queue = Arc::new(MockQueue::default());
    queue.enqueue_job(sample_job(7)).await;
    let processor = Arc::new(MockProcessor::new(Duration::from_millis(10)));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let handle = tokio::spawn(run_service(
        Arc::clone(&queue),
        Arc::clone(&processor),
        ServiceConfig {
            max_concurrent_jobs: 2,
            poll_timeout: Duration::from_millis(10),
        },
        shutdown_rx,
    ));

    wait_for_response_count(&queue, 1).await;
    shutdown_tx.send(true).expect("shutdown should send");
    handle
        .await
        .expect("service task should join")
        .expect("service should succeed");

    let responses = queue.responses().await;
    let response: ProverResponse =
        serde_json::from_str(&responses[0]).expect("response should deserialize");
    assert_eq!(response.request_id, [7; 32]);
    assert_eq!(response.result, vec![7, 8]);
    assert_eq!(response.proof, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn test_run_service_limits_concurrency() {
    let queue = Arc::new(MockQueue::default());
    for seed in 1..=4 {
        queue.enqueue_job(sample_job(seed)).await;
    }

    let processor = Arc::new(MockProcessor::new(Duration::from_millis(60)));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let handle = tokio::spawn(run_service(
        Arc::clone(&queue),
        Arc::clone(&processor),
        ServiceConfig {
            max_concurrent_jobs: 2,
            poll_timeout: Duration::from_millis(10),
        },
        shutdown_rx,
    ));

    wait_for_response_count(&queue, 4).await;
    shutdown_tx.send(true).expect("shutdown should send");
    handle
        .await
        .expect("service task should join")
        .expect("service should succeed");

    assert_eq!(processor.max_active(), 2);
}

#[tokio::test]
async fn test_run_service_drains_inflight_jobs_on_shutdown() {
    let queue = Arc::new(MockQueue::default());
    queue.enqueue_job(sample_job(11)).await;
    queue.enqueue_job(sample_job(12)).await;
    let processor = Arc::new(MockProcessor::new(Duration::from_millis(120)));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let handle = tokio::spawn(run_service(
        Arc::clone(&queue),
        Arc::clone(&processor),
        ServiceConfig {
            max_concurrent_jobs: 1,
            poll_timeout: Duration::from_millis(10),
        },
        shutdown_rx,
    ));

    timeout(Duration::from_secs(1), async {
        loop {
            if processor.started() >= 1 {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("first job should start");

    shutdown_tx.send(true).expect("shutdown should send");
    handle
        .await
        .expect("service task should join")
        .expect("service should succeed");

    let responses = queue.responses().await;
    assert_eq!(
        responses.len(),
        1,
        "only the in-flight job should complete after shutdown"
    );
    let response: ProverResponse =
        serde_json::from_str(&responses[0]).expect("response should deserialize");
    assert_eq!(response.request_id, [11; 32]);
}

#[tokio::test]
async fn test_run_service_logs_and_skips_failed_jobs() {
    let failing_job = sample_job(21);
    let succeeding_job = sample_job(22);
    let queue = Arc::new(MockQueue::default());
    queue.enqueue_job(failing_job.clone()).await;
    queue.enqueue_job(succeeding_job.clone()).await;
    let processor = Arc::new(MockProcessor::failing(
        Duration::from_millis(20),
        failing_job.request_id,
    ));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let handle = tokio::spawn(run_service(
        Arc::clone(&queue),
        Arc::clone(&processor),
        ServiceConfig {
            max_concurrent_jobs: 2,
            poll_timeout: Duration::from_millis(10),
        },
        shutdown_rx,
    ));

    wait_for_response_count(&queue, 1).await;
    shutdown_tx.send(true).expect("shutdown should send");
    handle
        .await
        .expect("service task should join")
        .expect("service should succeed");

    let responses = queue.responses().await;
    assert_eq!(responses.len(), 1);
    let response: ProverResponse =
        serde_json::from_str(&responses[0]).expect("response should deserialize");
    assert_eq!(response.request_id, succeeding_job.request_id);
}
