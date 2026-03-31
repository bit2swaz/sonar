//! Sonar ZK Coprocessor — Anchor program.

// Anchor macros emit cfg-flags that only exist on the sbf target; silence them
// when building for the native host (e.g. `cargo check`, `cargo test`).
#![cfg_attr(not(target_os = "solana"), allow(unexpected_cfgs))]
// Anchor's #[program] macro generates diverging expressions (e.g. via msg!/panic
// in dispatch code) that clippy flags. This is a known Anchor limitation.
#![allow(clippy::diverging_sub_expression)]

use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke,
    },
    system_program::{transfer, Transfer},
};
use groth16_solana::groth16::{Groth16Verifier, Groth16Verifyingkey};

mod verifier_registry;

use verifier_registry::{DEMO_COMPUTATION_ID, DEMO_PUBLIC_INPUTS_LEN, DEMO_VERIFYING_KEY};

declare_id!("EE2sQ2VRa1hY3qjPQ1PEwuPZX6dGwTZwHMCumWrGn3sV");

const MAX_RESULT_BYTES: usize = 10_000;
const GROTH16_PROOF_A_BYTES: usize = 64;
const GROTH16_PROOF_B_BYTES: usize = 128;
const GROTH16_PROOF_C_BYTES: usize = 64;
const GROTH16_PROOF_BYTES: usize =
    GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES + GROTH16_PROOF_C_BYTES;
const SONAR_CALLBACK_DISCRIMINATOR: [u8; 8] = [165, 188, 38, 190, 145, 138, 75, 149];

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

#[program]
pub mod sonar {
    use super::*;

    /// Submit a ZK computation request on-chain.
    pub fn request(ctx: Context<Request>, params: RequestParams) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        require!(params.deadline > current_slot, ErrorCode::DeadlinePassed);
        require!(params.fee > 0, ErrorCode::InsufficientFee);

        let request_metadata = &mut ctx.accounts.request_metadata;
        request_metadata.request_id = params.request_id;
        request_metadata.payer = ctx.accounts.payer.key();
        request_metadata.callback_program = ctx.accounts.callback_program.key();
        request_metadata.result_account = ctx.accounts.result_account.key();
        request_metadata.computation_id = params.computation_id;
        request_metadata.deadline = params.deadline;
        request_metadata.fee = params.fee;
        request_metadata.status = RequestStatus::Pending;
        request_metadata.completed_at = None;
        request_metadata.bump = ctx.bumps.request_metadata;

        let result_account = &mut ctx.accounts.result_account;
        result_account.request_id = params.request_id;
        result_account.result = Vec::new();
        result_account.is_set = false;
        result_account.written_at = None;
        result_account.bump = ctx.bumps.result_account;

        let transfer_accounts = Transfer {
            from: ctx.accounts.payer.to_account_info(),
            to: request_metadata.to_account_info(),
        };
        let transfer_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            transfer_accounts,
        );
        transfer(transfer_ctx, params.fee)?;

        // Emit structured log so the off-chain coordinator can detect new requests.
        // Format: "sonar:request:<64-char lowercase hex request_id>"
        let hex_id: String = params
            .request_id
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        msg!("sonar:request:{}", hex_id);

        Ok(())
    }

    /// Return a verified ZK proof + result for a pending request.
    pub fn callback<'info>(
        ctx: Context<'_, '_, '_, 'info, Callback<'info>>,
        params: CallbackParams,
    ) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        let request_metadata = &ctx.accounts.request_metadata;

        require!(
            request_metadata.status == RequestStatus::Pending,
            ErrorCode::RequestNotPending
        );
        require!(
            current_slot <= request_metadata.deadline,
            ErrorCode::DeadlinePassed
        );
        require!(
            !ctx.accounts.result_account.is_set,
            ErrorCode::AlreadyCompleted
        );

        verify_groth16_proof(request_metadata, &params)?;

        require!(
            params.result.len() <= MAX_RESULT_BYTES,
            ErrorCode::ResultTooLarge
        );

        let callback_result = params.result;
        let request_id = request_metadata.request_id;
        let fee = request_metadata.fee;

        {
            let result_account = &mut ctx.accounts.result_account;
            result_account.result = callback_result.clone();
            result_account.is_set = true;
            result_account.written_at = Some(current_slot);
        }

        {
            let request_metadata = &mut ctx.accounts.request_metadata;
            request_metadata.status = RequestStatus::Completed;
            request_metadata.completed_at = Some(current_slot);
        }

        invoke_callback_program(
            ctx.accounts.callback_program.to_account_info(),
            ctx.accounts.request_metadata.to_account_info(),
            ctx.accounts.result_account.to_account_info(),
            ctx.accounts.prover.to_account_info(),
            ctx.remaining_accounts.to_vec(),
            request_id,
            &callback_result,
        )?;

        move_lamports(
            &ctx.accounts.request_metadata.to_account_info(),
            &ctx.accounts.prover.to_account_info(),
            fee,
        )?;

        Ok(())
    }

    /// Reclaim the locked fee when a request expires past its deadline.
    pub fn refund(ctx: Context<Refund>) -> Result<()> {
        let request_metadata = &mut ctx.accounts.request_metadata;
        let current_slot = Clock::get()?.slot;

        move_lamports(
            &request_metadata.to_account_info(),
            &ctx.accounts.payer.to_account_info(),
            request_metadata.fee,
        )?;

        request_metadata.status = RequestStatus::Refunded;
        request_metadata.completed_at = Some(current_slot);

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Account contexts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(params: RequestParams)]
pub struct Request<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: Arbitrary callback program recorded in request metadata.
    #[account(constraint = callback_program.executable @ ErrorCode::CallbackProgramNotExecutable)]
    pub callback_program: UncheckedAccount<'info>,
    #[account(
        init,
        payer = payer,
        space = RequestMetadata::LEN,
        seeds = [b"request", params.request_id.as_ref()],
        bump
    )]
    pub request_metadata: Account<'info, RequestMetadata>,
    #[account(
        init,
        payer = payer,
        space = ResultAccount::LEN,
        seeds = [b"result", params.request_id.as_ref()],
        bump
    )]
    pub result_account: Account<'info, ResultAccount>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Callback<'info> {
    #[account(
        mut,
        seeds = [b"request", request_metadata.request_id.as_ref()],
        bump = request_metadata.bump,
        has_one = result_account @ ErrorCode::InvalidRequestId,
        has_one = callback_program @ ErrorCode::CallbackProgramMismatch,
    )]
    pub request_metadata: Account<'info, RequestMetadata>,
    #[account(
        mut,
        seeds = [b"result", request_metadata.request_id.as_ref()],
        bump = result_account.bump,
    )]
    pub result_account: Account<'info, ResultAccount>,
    #[account(mut)]
    pub prover: Signer<'info>,
    /// CHECK: Validated against request metadata via `has_one`.
    #[account(constraint = callback_program.executable @ ErrorCode::CallbackProgramNotExecutable)]
    pub callback_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct Refund<'info> {
    #[account(
        mut,
        seeds = [b"request", request_metadata.request_id.as_ref()],
        bump = request_metadata.bump,
        has_one = payer @ ErrorCode::RefundPayerMismatch,
        constraint = request_metadata.status == RequestStatus::Pending @ ErrorCode::RequestNotPending,
        constraint = Clock::get()?.slot > request_metadata.deadline @ ErrorCode::DeadlineNotReached,
    )]
    pub request_metadata: Account<'info, RequestMetadata>,
    #[account(mut)]
    pub payer: Signer<'info>,
}

#[account]
pub struct RequestMetadata {
    pub request_id: [u8; 32],
    pub payer: Pubkey,
    pub callback_program: Pubkey,
    pub result_account: Pubkey,
    pub computation_id: [u8; 32],
    pub deadline: u64,
    pub fee: u64,
    pub status: RequestStatus,
    pub completed_at: Option<u64>,
    pub bump: u8,
}

impl RequestMetadata {
    pub const LEN: usize = 8 + 32 + 32 + 32 + 32 + 32 + 8 + 8 + 1 + 9 + 1;
}

#[account]
pub struct ResultAccount {
    pub request_id: [u8; 32],
    pub result: Vec<u8>,
    pub is_set: bool,
    pub written_at: Option<u64>,
    pub bump: u8,
}

impl ResultAccount {
    pub const LEN: usize = 8 + 32 + 4 + MAX_RESULT_BYTES + 1 + 9 + 1;
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

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct SonarCallbackPayload {
    pub request_id: [u8; 32],
    pub result: Vec<u8>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Completed,
    Refunded,
}

fn verify_groth16_proof(request_metadata: &RequestMetadata, params: &CallbackParams) -> Result<()> {
    match request_metadata.computation_id {
        DEMO_COMPUTATION_ID => {
            verify_with_key::<DEMO_PUBLIC_INPUTS_LEN>(&DEMO_VERIFYING_KEY, params)
        },
        _ => Err(error!(ErrorCode::UnknownComputationId)),
    }
}

fn verify_with_key<const N: usize>(
    verifying_key: &Groth16Verifyingkey<'static>,
    params: &CallbackParams,
) -> Result<()> {
    let proof_bytes: [u8; GROTH16_PROOF_BYTES] = params
        .proof
        .as_slice()
        .try_into()
        .map_err(|_| error!(ErrorCode::InvalidProofLength))?;
    let proof_a: [u8; GROTH16_PROOF_A_BYTES] = proof_bytes[..GROTH16_PROOF_A_BYTES]
        .try_into()
        .map_err(|_| error!(ErrorCode::InvalidProofLength))?;
    let proof_b: [u8; GROTH16_PROOF_B_BYTES] = proof_bytes
        [GROTH16_PROOF_A_BYTES..GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES]
        .try_into()
        .map_err(|_| error!(ErrorCode::InvalidProofLength))?;
    let proof_c: [u8; GROTH16_PROOF_C_BYTES] = proof_bytes
        [GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES..]
        .try_into()
        .map_err(|_| error!(ErrorCode::InvalidProofLength))?;

    let public_inputs = parse_public_inputs::<N>(&params.public_inputs)?;
    let mut verifier =
        Groth16Verifier::new(&proof_a, &proof_b, &proof_c, &public_inputs, verifying_key)
            .map_err(|_| error!(ErrorCode::ProofVerificationFailed))?;

    verifier
        .verify()
        .map_err(|_| error!(ErrorCode::ProofVerificationFailed))
}

fn parse_public_inputs<const N: usize>(public_inputs: &[Vec<u8>]) -> Result<[[u8; 32]; N]> {
    require!(
        public_inputs.len() == N,
        ErrorCode::InvalidPublicInputsLength
    );

    let mut parsed = [[0u8; 32]; N];
    for (index, input) in public_inputs.iter().enumerate() {
        parsed[index] = input
            .as_slice()
            .try_into()
            .map_err(|_| error!(ErrorCode::InvalidPublicInputSize))?;
    }

    Ok(parsed)
}

fn invoke_callback_program<'info>(
    callback_program: AccountInfo<'info>,
    request_metadata: AccountInfo<'info>,
    result_account: AccountInfo<'info>,
    prover: AccountInfo<'info>,
    remaining_accounts: Vec<AccountInfo<'info>>,
    request_id: [u8; 32],
    result: &[u8],
) -> Result<()> {
    let payload = SonarCallbackPayload {
        request_id,
        result: result.to_vec(),
    };

    let mut data = SONAR_CALLBACK_DISCRIMINATOR.to_vec();
    data.extend_from_slice(&payload.try_to_vec()?);

    let mut accounts = vec![
        AccountMeta::new_readonly(*request_metadata.key, false),
        AccountMeta::new_readonly(*result_account.key, false),
        AccountMeta::new_readonly(*prover.key, true),
    ];
    let mut account_infos = vec![
        callback_program.clone(),
        request_metadata,
        result_account,
        prover,
    ];

    for account in remaining_accounts {
        accounts.push(if account.is_writable {
            AccountMeta::new(*account.key, account.is_signer)
        } else {
            AccountMeta::new_readonly(*account.key, account.is_signer)
        });
        account_infos.push(account.clone());
    }

    invoke(
        &Instruction {
            program_id: *callback_program.key,
            accounts,
            data,
        },
        &account_infos,
    )
    .map_err(|_| error!(ErrorCode::CallbackInvokeFailed))
}

fn move_lamports(from: &AccountInfo<'_>, to: &AccountInfo<'_>, amount: u64) -> Result<()> {
    let updated_from = from
        .lamports()
        .checked_sub(amount)
        .ok_or_else(|| error!(ErrorCode::InsufficientFee))?;
    let updated_to = to
        .lamports()
        .checked_add(amount)
        .ok_or_else(|| error!(ErrorCode::LamportOverflow))?;

    **from.try_borrow_mut_lamports()? = updated_from;
    **to.try_borrow_mut_lamports()? = updated_to;
    Ok(())
}

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[error_code]
pub enum ErrorCode {
    #[msg("Request deadline already passed")]
    DeadlinePassed,
    #[msg("Request deadline has not been reached")]
    DeadlineNotReached,
    #[msg("Request already completed")]
    AlreadyCompleted,
    #[msg("Request is not pending")]
    RequestNotPending,
    #[msg("Proof verification failed")]
    ProofVerificationFailed,
    #[msg("Invalid request ID")]
    InvalidRequestId,
    #[msg("Callback program does not match")]
    CallbackProgramMismatch,
    #[msg("Callback program must be executable")]
    CallbackProgramNotExecutable,
    #[msg("Callback CPI failed")]
    CallbackInvokeFailed,
    #[msg("Insufficient fee")]
    InsufficientFee,
    #[msg("Refund caller does not match the original payer")]
    RefundPayerMismatch,
    #[msg("No verifier is registered for this computation ID")]
    UnknownComputationId,
    #[msg("Groth16 proof length is invalid")]
    InvalidProofLength,
    #[msg("Public inputs length is invalid for the configured verifier")]
    InvalidPublicInputsLength,
    #[msg("Each public input must be exactly 32 bytes")]
    InvalidPublicInputSize,
    #[msg("Result payload exceeds the configured size limit")]
    ResultTooLarge,
    #[msg("Lamport arithmetic overflowed")]
    LamportOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_public_inputs_accepts_exact_32_byte_rows() {
        let parsed = parse_public_inputs::<2>(&[vec![1u8; 32], vec![2u8; 32]]).unwrap();
        assert_eq!(parsed[0], [1u8; 32]);
        assert_eq!(parsed[1], [2u8; 32]);
    }

    #[test]
    fn parse_public_inputs_rejects_wrong_row_width() {
        let err = parse_public_inputs::<1>(&[vec![7u8; 31]]).unwrap_err();
        assert_eq!(err, error!(ErrorCode::InvalidPublicInputSize));
    }
}
