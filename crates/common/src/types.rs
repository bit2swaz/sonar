//! Shared domain types for the Sonar ZK coprocessor.
//!
//! `Pubkey` is represented as `[u8; 32]` so this crate has no dependency on
//! `solana-program`.  Crates that work with real Solana pubkeys can convert
//! with `pubkey.to_bytes()` / `Pubkey::new_from_array(bytes)`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Pubkey newtype
// ---------------------------------------------------------------------------

/// A 32-byte Solana public key, represented without a Solana crate dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Pubkey(pub [u8; 32]);

impl Pubkey {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// ComputationId
// ---------------------------------------------------------------------------

/// A 32-byte identifier for an SP1 computation (hash of the ELF binary).
pub type ComputationId = [u8; 32];

// ---------------------------------------------------------------------------
// CallbackAccountMeta
// ---------------------------------------------------------------------------

/// Additional account metadata that should be replayed during callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallbackAccountMeta {
    pub pubkey: Pubkey,
    pub is_writable: bool,
}

// ---------------------------------------------------------------------------
// RequestStatus
// ---------------------------------------------------------------------------

/// On-chain status of a computation request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestStatus {
    Pending,
    Completed,
    Refunded,
}

// ---------------------------------------------------------------------------
// RequestMetadata
// ---------------------------------------------------------------------------

/// Mirror of the on-chain `RequestMetadata` Anchor account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub request_id: [u8; 32],
    pub payer: Pubkey,
    pub callback_program: Pubkey,
    pub computation_id: [u8; 32],
    pub deadline: u64,
    pub fee: u64,
    pub status: RequestStatus,
    pub bump: u8,
}

// ---------------------------------------------------------------------------
// RequestParams
// ---------------------------------------------------------------------------

/// Instruction data for the `request` instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestParams {
    pub request_id: [u8; 32],
    pub computation_id: [u8; 32],
    pub inputs: Vec<u8>,
    pub deadline: u64,
    pub fee: u64,
}

// ---------------------------------------------------------------------------
// CallbackParams
// ---------------------------------------------------------------------------

/// Instruction data for the `callback` instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallbackParams {
    pub proof: Vec<u8>,
    pub public_inputs: Vec<Vec<u8>>,
    pub result: Vec<u8>,
}

// ---------------------------------------------------------------------------
// ComputationRequest / ComputationResult
// ---------------------------------------------------------------------------

/// An off-chain computation request handed to the prover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputationRequest {
    pub id: ComputationId,
    pub inputs: Vec<u8>,
    pub deadline: u64,
    pub fee: u64,
}

/// The result produced by the prover for a given request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputationResult {
    pub request_id: [u8; 32],
    pub result: Vec<u8>,
    pub proof: Vec<u8>,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// ProofVerificationResult
// ---------------------------------------------------------------------------

/// Outcome of on-chain Groth16 proof verification.
#[derive(Debug, Clone, PartialEq)]
pub enum ProofVerificationResult {
    Success { result: Vec<u8> },
    Failure { reason: String },
}

impl ProofVerificationResult {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    pub fn result(&self) -> Option<&[u8]> {
        match self {
            Self::Success { result } => Some(result),
            Self::Failure { .. } => None,
        }
    }
}

// ---------------------------------------------------------------------------
// ProverJob / ProverResponse
// ---------------------------------------------------------------------------

/// A job dispatched from the coordinator to the prover via Redis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProverJob {
    pub request_id: [u8; 32],
    pub computation_id: ComputationId,
    pub inputs: Vec<u8>,
    pub deadline: u64,
    pub fee: u64,
    pub callback_program: Pubkey,
    pub result_account: Pubkey,
    #[serde(default)]
    pub callback_accounts: Vec<CallbackAccountMeta>,
}

/// The response the prover pushes back to the coordinator via Redis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProverResponse {
    pub request_id: [u8; 32],
    pub result: Vec<u8>,
    pub proof: Vec<u8>,
    pub public_inputs: Vec<Vec<u8>>,
    /// Simulated compute-unit usage, used for metrics.
    pub gas_used: u64,
    #[serde(default)]
    pub callback_accounts: Vec<CallbackAccountMeta>,
}

// ---------------------------------------------------------------------------
// GasEstimate
// ---------------------------------------------------------------------------

/// A CU cost estimate for submitting a callback transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub cu_units: u64,
    pub cu_price: u64,
    pub total_cost_lamports: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- helpers ---

    fn sample_id() -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = 0xDE;
        id[31] = 0xAD;
        id
    }

    fn sample_pubkey(seed: u8) -> Pubkey {
        Pubkey::new([seed; 32])
    }

    fn sample_callback_account(seed: u8, is_writable: bool) -> CallbackAccountMeta {
        CallbackAccountMeta {
            pubkey: sample_pubkey(seed),
            is_writable,
        }
    }

    // --- RequestMetadata ---

    #[test]
    fn test_request_metadata_construction() {
        let meta = RequestMetadata {
            request_id: sample_id(),
            payer: sample_pubkey(1),
            callback_program: sample_pubkey(2),
            computation_id: [0xAB; 32],
            deadline: 1_000_000,
            fee: 5_000,
            status: RequestStatus::Pending,
            bump: 255,
        };
        assert_eq!(meta.request_id[0], 0xDE);
        assert_eq!(meta.request_id[31], 0xAD);
        assert_eq!(meta.payer, sample_pubkey(1));
        assert_eq!(meta.callback_program, sample_pubkey(2));
        assert_eq!(meta.computation_id, [0xAB; 32]);
        assert_eq!(meta.deadline, 1_000_000);
        assert_eq!(meta.fee, 5_000);
        assert_eq!(meta.status, RequestStatus::Pending);
        assert_eq!(meta.bump, 255);
    }

    #[test]
    fn test_request_metadata_serde_roundtrip() {
        let meta = RequestMetadata {
            request_id: sample_id(),
            payer: sample_pubkey(3),
            callback_program: sample_pubkey(4),
            computation_id: [0xCD; 32],
            deadline: 99,
            fee: 1,
            status: RequestStatus::Completed,
            bump: 1,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let decoded: RequestMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, decoded);
    }

    // --- RequestStatus ---

    #[test]
    fn test_request_status_serde_pending() {
        let json = serde_json::to_string(&RequestStatus::Pending).unwrap();
        assert_eq!(json, r#""pending""#);
    }

    #[test]
    fn test_request_status_serde_completed() {
        let json = serde_json::to_string(&RequestStatus::Completed).unwrap();
        assert_eq!(json, r#""completed""#);
    }

    #[test]
    fn test_request_status_serde_refunded() {
        let json = serde_json::to_string(&RequestStatus::Refunded).unwrap();
        assert_eq!(json, r#""refunded""#);
    }

    #[test]
    fn test_request_status_deserialize() {
        let status: RequestStatus = serde_json::from_str(r#""pending""#).unwrap();
        assert_eq!(status, RequestStatus::Pending);
    }

    // --- RequestParams ---

    #[test]
    fn test_request_params_serde_roundtrip() {
        let params = RequestParams {
            request_id: sample_id(),
            computation_id: [0xAB; 32],
            inputs: vec![1, 2, 3, 4],
            deadline: 9_999,
            fee: 500,
        };
        let json = serde_json::to_string(&params).unwrap();
        let decoded: RequestParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params, decoded);
    }

    // --- CallbackParams ---

    #[test]
    fn test_callback_params_serde_roundtrip() {
        let params = CallbackParams {
            proof: vec![0xFF; 128],
            public_inputs: vec![vec![1, 2], vec![3, 4]],
            result: vec![42],
        };
        let json = serde_json::to_string(&params).unwrap();
        let decoded: CallbackParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params, decoded);
    }

    // --- ComputationResult ---

    #[test]
    fn test_computation_result_serde_roundtrip() {
        let cr = ComputationResult {
            request_id: sample_id(),
            result: vec![7, 8, 9],
            proof: vec![0xAA; 64],
            timestamp: 1_700_000_000,
        };
        let json = serde_json::to_string(&cr).unwrap();
        let decoded: ComputationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(cr, decoded);
    }

    // --- ProofVerificationResult ---

    #[test]
    fn test_verification_success_is_success() {
        let pvr = ProofVerificationResult::Success {
            result: vec![1, 2, 3],
        };
        assert!(pvr.is_success());
    }

    #[test]
    fn test_verification_failure_is_not_success() {
        let pvr = ProofVerificationResult::Failure {
            reason: "bad proof".to_string(),
        };
        assert!(!pvr.is_success());
    }

    #[test]
    fn test_verification_success_result() {
        let expected = vec![10, 20, 30];
        let pvr = ProofVerificationResult::Success {
            result: expected.clone(),
        };
        assert_eq!(pvr.result(), Some(expected.as_slice()));
    }

    #[test]
    fn test_verification_failure_result_is_none() {
        let pvr = ProofVerificationResult::Failure {
            reason: "invalid".to_string(),
        };
        assert_eq!(pvr.result(), None);
    }

    // --- ProverJob ---

    #[test]
    fn test_prover_job_serde_roundtrip() {
        let job = ProverJob {
            request_id: sample_id(),
            computation_id: [0x01; 32],
            inputs: vec![0x10, 0x20],
            deadline: 12345,
            fee: 9000,
            callback_program: sample_pubkey(5),
            result_account: sample_pubkey(6),
            callback_accounts: vec![sample_callback_account(7, true)],
        };
        let json = serde_json::to_string(&job).unwrap();
        let decoded: ProverJob = serde_json::from_str(&json).unwrap();
        assert_eq!(job.request_id, decoded.request_id);
        assert_eq!(job.computation_id, decoded.computation_id);
        assert_eq!(job.inputs, decoded.inputs);
        assert_eq!(job.deadline, decoded.deadline);
        assert_eq!(job.fee, decoded.fee);
        assert_eq!(job.callback_program, decoded.callback_program);
        assert_eq!(job.result_account, decoded.result_account);
        assert_eq!(job.callback_accounts, decoded.callback_accounts);
    }

    #[test]
    fn test_prover_job_deserializes_missing_callback_accounts_as_empty() {
        let json = serde_json::json!({
            "request_id": sample_id(),
            "computation_id": vec![0x01; 32],
            "inputs": [0x10, 0x20],
            "deadline": 12345,
            "fee": 9000,
            "callback_program": sample_pubkey(5),
            "result_account": sample_pubkey(6),
        })
        .to_string();

        let decoded: ProverJob = serde_json::from_str(&json).unwrap();
        assert!(decoded.callback_accounts.is_empty());
    }

    // --- ProverResponse ---

    #[test]
    fn test_prover_response_serde_roundtrip() {
        let resp = ProverResponse {
            request_id: sample_id(),
            result: vec![55],
            proof: vec![0xBB; 32],
            public_inputs: vec![vec![0x37; 8]],
            gas_used: 200_000,
            callback_accounts: vec![sample_callback_account(8, false)],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: ProverResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.request_id, decoded.request_id);
        assert_eq!(resp.result, decoded.result);
        assert_eq!(resp.proof, decoded.proof);
        assert_eq!(resp.public_inputs, decoded.public_inputs);
        assert_eq!(resp.gas_used, decoded.gas_used);
        assert_eq!(resp.callback_accounts, decoded.callback_accounts);
    }

    #[test]
    fn test_prover_response_deserializes_missing_callback_accounts_as_empty() {
        let json = serde_json::json!({
            "request_id": sample_id(),
            "result": [55],
            "proof": [187, 187, 187],
            "public_inputs": [[55]],
            "gas_used": 200000,
        })
        .to_string();

        let decoded: ProverResponse = serde_json::from_str(&json).unwrap();
        assert!(decoded.callback_accounts.is_empty());
    }

    // --- GasEstimate ---

    #[test]
    fn test_gas_estimate_total() {
        let cu_units: u64 = 200_000;
        let cu_price: u64 = 1_000; // micro-lamports per CU
        let total_cost_lamports = cu_units * cu_price;
        let est = GasEstimate {
            cu_units,
            cu_price,
            total_cost_lamports,
        };
        assert_eq!(est.total_cost_lamports, est.cu_units * est.cu_price);
    }

    #[test]
    fn test_gas_estimate_serde_roundtrip() {
        let est = GasEstimate {
            cu_units: 150_000,
            cu_price: 500,
            total_cost_lamports: 75_000_000,
        };
        let json = serde_json::to_string(&est).unwrap();
        let decoded: GasEstimate = serde_json::from_str(&json).unwrap();
        assert_eq!(est.cu_units, decoded.cu_units);
        assert_eq!(est.cu_price, decoded.cu_price);
        assert_eq!(est.total_cost_lamports, decoded.total_cost_lamports);
    }

    // --- Pubkey ---

    #[test]
    fn test_pubkey_default_is_zeroed() {
        assert_eq!(Pubkey::default().0, [0u8; 32]);
    }

    #[test]
    fn test_pubkey_as_bytes() {
        let pk = Pubkey::new([0xCC; 32]);
        assert_eq!(pk.as_bytes(), &[0xCC; 32]);
    }

    #[test]
    fn test_pubkey_serde_roundtrip() {
        let pk = Pubkey::new([0x11; 32]);
        let json = serde_json::to_string(&pk).unwrap();
        let decoded: Pubkey = serde_json::from_str(&json).unwrap();
        assert_eq!(pk, decoded);
    }
}
