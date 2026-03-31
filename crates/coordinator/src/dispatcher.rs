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
            let resp =
                serde_json::from_str(&payload).context("deserialise ProverResponse")?;
            Ok(Some(resp))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sonar_common::types::Pubkey;

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
}

