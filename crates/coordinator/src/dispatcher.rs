//! Dispatcher — thin wrappers around Redis list operations.
//!
//! The coordinator listener pushes [`ProverJob`]s onto `sonar:jobs`.
//! The prover pops jobs, runs the proof, and pushes [`ProverResponse`]s onto
//! `sonar:responses`.  The coordinator callback worker pops from that queue.

use anyhow::Context as _;
use redis::AsyncCommands;
use sonar_common::types::{ProverJob, ProverResponse};

// ---------------------------------------------------------------------------
// Queue names
// ---------------------------------------------------------------------------

pub const JOBS_QUEUE: &str = "sonar:jobs";
pub const RESPONSES_QUEUE: &str = "sonar:responses";

// ---------------------------------------------------------------------------
// Push helpers
// ---------------------------------------------------------------------------

/// Serialise `job` as JSON and RPUSH it onto the back of `queue`.
pub async fn push_job<C: AsyncCommands>(
    conn: &mut C,
    queue: &str,
    job: &ProverJob,
) -> anyhow::Result<()> {
    let payload = serde_json::to_string(job).context("serialise ProverJob")?;
    conn.rpush::<_, _, ()>(queue, payload)
        .await
        .context("redis RPUSH jobs")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pop helpers
// ---------------------------------------------------------------------------

/// Blocking-pop a [`ProverResponse`] from `queue` with the given timeout.
/// Returns `None` when the timeout elapses before any item arrives.
pub async fn pop_response<C: AsyncCommands>(
    conn: &mut C,
    queue: &str,
    timeout_secs: f64,
) -> anyhow::Result<Option<ProverResponse>> {
    let result: Option<(String, String)> = conn
        .blpop(queue, timeout_secs)
        .await
        .context("redis BLPOP responses")?;
    match result {
        None => Ok(None),
        Some((_key, payload)) => {
            let resp = serde_json::from_str(&payload).context("deserialise ProverResponse")?;
            Ok(Some(resp))
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use redis::AsyncCommands;
    use sonar_common::types::Pubkey;
    use testcontainers::{
        core::{IntoContainerPort, WaitFor},
        runners::AsyncRunner,
        GenericImage, ImageExt,
    };

    fn sample_job() -> ProverJob {
        ProverJob {
            request_id: [1u8; 32],
            computation_id: [2u8; 32],
            inputs: vec![3, 4, 5],
            deadline: 1_000,
            fee: 500,
            callback_program: Pubkey::new([6u8; 32]),
            result_account: Pubkey::new([7u8; 32]),
        }
    }

    fn sample_response() -> ProverResponse {
        ProverResponse {
            request_id: [1u8; 32],
            result: vec![42],
            proof: vec![0u8; 32],
            public_inputs: vec![vec![42]],
            gas_used: 200_000,
        }
    }

    #[test]
    fn job_serde_roundtrip() {
        let job = sample_job();
        let json = serde_json::to_string(&job).expect("serialize");
        let decoded: ProverJob = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.request_id, job.request_id);
        assert_eq!(decoded.inputs, job.inputs);
        assert_eq!(decoded.deadline, job.deadline);
        assert_eq!(decoded.callback_program, job.callback_program);
    }

    #[test]
    fn response_serde_roundtrip() {
        let resp = sample_response();
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: ProverResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.request_id, resp.request_id);
        assert_eq!(decoded.result, resp.result);
        assert_eq!(decoded.public_inputs, resp.public_inputs);
        assert_eq!(decoded.gas_used, resp.gas_used);
    }

    #[test]
    fn job_with_empty_inputs_roundtrips() {
        let mut job = sample_job();
        job.inputs = vec![];
        let json = serde_json::to_string(&job).unwrap();
        let decoded: ProverJob = serde_json::from_str(&json).unwrap();
        assert!(decoded.inputs.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pop_response_returns_none_after_timeout() {
        let redis = GenericImage::new("redis", "7.2.4")
            .with_exposed_port(6379.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
            .with_startup_timeout(Duration::from_secs(60))
            .start()
            .await
            .expect("redis testcontainer should start");

        let redis_url = format!(
            "redis://{}:{}",
            redis.get_host().await.expect("redis host"),
            redis
                .get_host_port_ipv4(6379.tcp())
                .await
                .expect("redis mapped port")
        );

        let client = redis::Client::open(redis_url.as_str()).expect("redis client should build");
        let mut connection = client
            .get_multiplexed_async_connection()
            .await
            .expect("redis connection should open");

        let queue = "sonar:responses:timeout-test";
        let _: () = connection
            .del(queue)
            .await
            .expect("queue cleanup should succeed");

        let response = pop_response(&mut connection, queue, 0.1)
            .await
            .expect("BLPOP timeout should not fail");

        assert!(response.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pop_response_rejects_malformed_payload() {
        let redis = GenericImage::new("redis", "7.2.4")
            .with_exposed_port(6379.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
            .with_startup_timeout(Duration::from_secs(60))
            .start()
            .await
            .expect("redis testcontainer should start");

        let redis_url = format!(
            "redis://{}:{}",
            redis.get_host().await.expect("redis host"),
            redis
                .get_host_port_ipv4(6379.tcp())
                .await
                .expect("redis mapped port")
        );

        let client = redis::Client::open(redis_url.as_str()).expect("redis client should build");
        let mut connection = client
            .get_multiplexed_async_connection()
            .await
            .expect("redis connection should open");

        let queue = "sonar:responses:malformed-test";
        let _: () = connection
            .del(queue)
            .await
            .expect("queue cleanup should succeed");
        connection
            .rpush::<_, _, ()>(queue, "{definitely-not-json")
            .await
            .expect("malformed response should enqueue");

        let error = pop_response(&mut connection, queue, 1.0)
            .await
            .expect_err("malformed payload should fail deserialization");

        assert!(
            error.to_string().contains("deserialise ProverResponse"),
            "unexpected error: {error:#}"
        );
    }
}
