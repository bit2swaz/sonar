//! Sonar ZK Coprocessor — Anchor program (Phase 2.1 placeholder).
//!
//! Full instruction logic is implemented in subsequent mini-phases.
//! Every instruction path is exercised by the integration tests in
//! `program/tests/sonar.ts`.

// Anchor macros emit cfg-flags that only exist on the sbf target; silence them
// when building for the native host (e.g. `cargo check`, `cargo test`).
#![cfg_attr(not(target_os = "solana"), allow(unexpected_cfgs))]
// Anchor's #[program] macro generates diverging expressions (e.g. via msg!/panic
// in dispatch code) that clippy flags. This is a known Anchor limitation.
#![allow(clippy::diverging_sub_expression)]

use anchor_lang::prelude::*;

declare_id!("5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84");

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

#[program]
pub mod sonar {
    use super::*;

    /// Submit a ZK computation request on-chain.
    pub fn request(_ctx: Context<Request>, _params: RequestParams) -> Result<()> {
        Ok(())
    }

    /// Return a verified ZK proof + result for a pending request.
    pub fn callback(_ctx: Context<Callback>, _params: CallbackParams) -> Result<()> {
        Ok(())
    }

    /// Reclaim the locked fee when a request expires past its deadline.
    pub fn refund(_ctx: Context<Refund>) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Account contexts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Request<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Callback<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

/// Parameters for the `request` instruction.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct RequestParams {
    /// Unique nonce chosen by the caller.
    pub request_id: [u8; 32],
    /// Hash identifying the zkVM image or circuit.
    pub computation_id: [u8; 32],
    /// Serialised inputs passed to the prover.
    pub inputs: Vec<u8>,
    /// Slot number after which the request expires.
    pub deadline: u64,
    /// Fee attached to this request (lamports or $SONAR).
    pub fee: u64,
}

/// Parameters for the `callback` instruction.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct CallbackParams {
    /// Groth16 proof bytes.
    pub proof: Vec<u8>,
    /// Public inputs for on-chain verification.
    pub public_inputs: Vec<Vec<u8>>,
    /// Result written to the result account.
    pub result: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[error_code]
pub enum ErrorCode {
    #[msg("Request deadline already passed")]
    DeadlinePassed,
    #[msg("Request already completed")]
    AlreadyCompleted,
    #[msg("Proof verification failed")]
    ProofVerificationFailed,
    #[msg("Invalid request ID")]
    InvalidRequestId,
    #[msg("Callback program does not match")]
    CallbackProgramMismatch,
    #[msg("Insufficient fee")]
    InsufficientFee,
}
