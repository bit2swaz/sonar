//! Sonar ZK Coprocessor — Anchor program.

// Anchor macros emit cfg-flags that only exist on the sbf target; silence them
// when building for the native host (e.g. `cargo check`, `cargo test`).
#![cfg_attr(not(target_os = "solana"), allow(unexpected_cfgs))]
// Anchor's #[program] macro generates diverging expressions (e.g. via msg!/panic
// in dispatch code) that clippy flags. This is a known Anchor limitation.
#![allow(clippy::diverging_sub_expression)]

use anchor_lang::prelude::*;
#[cfg(target_os = "solana")]
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke,
};
#[cfg(target_os = "solana")]
use anchor_lang::system_program::{transfer, Transfer};
use groth16_solana::groth16::{Groth16Verifier, Groth16Verifyingkey};

mod instructions;
mod verifier_registry;

pub use verifier_registry::{VerifierRegistry, DEMO_COMPUTATION_ID, HISTORICAL_AVG_COMPUTATION_ID};

declare_id!("Gf7RSZYmfNJ5kv2AJvcv5rjCANP6ePExJR19D91MECLY");

const MAX_RESULT_BYTES: usize = 10_000;
const GROTH16_PROOF_A_BYTES: usize = 64;
const GROTH16_PROOF_B_BYTES: usize = 128;
const GROTH16_PROOF_C_BYTES: usize = 64;
const GROTH16_PROOF_BYTES: usize =
    GROTH16_PROOF_A_BYTES + GROTH16_PROOF_B_BYTES + GROTH16_PROOF_C_BYTES;
#[cfg(target_os = "solana")]
const SONAR_CALLBACK_DISCRIMINATOR: [u8; 8] = [165, 188, 38, 190, 145, 138, 75, 149];

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

#[program]
pub mod sonar {
    use super::*;

    pub fn register_verifier(
        ctx: Context<RegisterVerifier>,
        params: RegisterVerifierParams,
    ) -> Result<()> {
        instructions::register_verifier::handler(ctx, params)
    }

    pub fn update_verifier(
        ctx: Context<UpdateVerifier>,
        params: RegisterVerifierParams,
    ) -> Result<()> {
        instructions::update_verifier::handler(ctx, params)
    }

    /// Submit a ZK computation request on-chain.
    pub fn request(ctx: Context<Request>, params: RequestParams) -> Result<()> {
        let current_slot = Clock::get()?.slot;
        require!(params.deadline > current_slot, ErrorCode::DeadlinePassed);
        require!(params.fee > 0, ErrorCode::InsufficientFee);

        let request_metadata = &mut ctx.accounts.request_metadata;
        request_metadata.request_id = params.request_id;
        request_metadata.payer = ctx.accounts.payer.key();
        request_metadata.callback_program = ctx.accounts.callback_program.key();
        request_metadata.computation_id = params.computation_id;
        request_metadata.deadline = params.deadline;
        request_metadata.fee = params.fee;
        request_metadata.status = RequestStatus::Pending;
        request_metadata.bump = ctx.bumps.request_metadata;

        let result_account = &mut ctx.accounts.result_account;
        result_account.request_id = params.request_id;
        result_account.bump = ctx.bumps.result_account;

        #[cfg(not(target_os = "solana"))]
        move_lamports(
            &ctx.accounts.payer.to_account_info(),
            &request_metadata.to_account_info(),
            params.fee,
        )?;

        #[cfg(target_os = "solana")]
        {
            let transfer_accounts = Transfer {
                from: ctx.accounts.payer.to_account_info(),
                to: request_metadata.to_account_info(),
            };
            let transfer_ctx = CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                transfer_accounts,
            );
            transfer(transfer_ctx, params.fee)?;
        }

        // Emit structured log so the off-chain coordinator can detect new requests.
        // Format: "sonar:request:<64-char lowercase hex request_id>"
        let hex_id: String = params
            .request_id
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        msg!("sonar:request:{}", hex_id);

        // Emit serialised inputs so the coordinator can build the prover job
        // without fetching the full transaction.
        // Format: "sonar:inputs:<lowercase hex-encoded bytes>"
        let hex_inputs: String = params.inputs.iter().map(|b| format!("{:02x}", b)).collect();
        msg!("sonar:inputs:{}", hex_inputs);

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
        require_keys_eq!(
            ctx.accounts.payer.key(),
            request_metadata.payer,
            ErrorCode::RequestPayerMismatch
        );
        require!(
            ctx.accounts.result_account.request_id == request_metadata.request_id,
            ErrorCode::InvalidRequestId
        );

        verify_groth16_proof(&ctx.accounts.verifier_registry, &params)?;

        validate_result_size(&params.result)?;

        let callback_result = params.result;
        let request_id = request_metadata.request_id;
        let fee = request_metadata.fee;

        {
            let request_metadata = &mut ctx.accounts.request_metadata;
            request_metadata.status = RequestStatus::Completed;
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

        require!(
            ctx.accounts.result_account.request_id == request_metadata.request_id,
            ErrorCode::InvalidRequestId
        );

        move_lamports(
            &request_metadata.to_account_info(),
            &ctx.accounts.payer.to_account_info(),
            request_metadata.fee,
        )?;

        request_metadata.status = RequestStatus::Refunded;

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
        has_one = payer @ ErrorCode::RequestPayerMismatch,
        has_one = callback_program @ ErrorCode::CallbackProgramMismatch,
        close = payer,
    )]
    pub request_metadata: Account<'info, RequestMetadata>,
    #[account(
        mut,
        seeds = [b"result", result_account.request_id.as_ref()],
        bump = result_account.bump,
        close = payer,
    )]
    pub result_account: Account<'info, ResultAccount>,
    #[account(
        seeds = [b"verifier", request_metadata.computation_id.as_ref()],
        bump = verifier_registry.bump,
        constraint = verifier_registry.computation_id == request_metadata.computation_id @ ErrorCode::UnknownComputationId,
    )]
    pub verifier_registry: Account<'info, VerifierRegistry>,
    #[account(mut)]
    pub prover: Signer<'info>,
    #[account(mut)]
    pub payer: SystemAccount<'info>,
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
        close = payer,
    )]
    pub request_metadata: Account<'info, RequestMetadata>,
    #[account(
        mut,
        seeds = [b"result", result_account.request_id.as_ref()],
        bump = result_account.bump,
        close = payer,
    )]
    pub result_account: Account<'info, ResultAccount>,
    #[account(mut)]
    pub payer: Signer<'info>,
}

#[derive(Accounts)]
#[instruction(params: RegisterVerifierParams)]
pub struct RegisterVerifier<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = VerifierRegistry::space_for(params.vk_ic.len()),
        seeds = [b"verifier", params.computation_id.as_ref()],
        bump
    )]
    pub verifier_registry: Account<'info, VerifierRegistry>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(params: RegisterVerifierParams)]
pub struct UpdateVerifier<'info> {
    /// Must match the authority stored in the existing registry.
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"verifier", verifier_registry.computation_id.as_ref()],
        bump = verifier_registry.bump,
        has_one = authority @ ErrorCode::VerifierAuthorityMismatch,
        realloc = VerifierRegistry::space_for(params.vk_ic.len()),
        realloc::payer = payer,
        realloc::zero = false,
    )]
    pub verifier_registry: Account<'info, VerifierRegistry>,
    /// Pays for any additional rent when the account grows.
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
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

impl RequestMetadata {
    pub const LEN: usize = 8 + 32 + 32 + 32 + 32 + 8 + 8 + 1 + 1;
}

#[account]
pub struct ResultAccount {
    pub request_id: [u8; 32],
    pub bump: u8,
}

impl ResultAccount {
    pub const LEN: usize = 8 + 32 + 1;
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

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct RegisterVerifierParams {
    pub computation_id: [u8; 32],
    pub vk_alpha_g1: [u8; 64],
    pub vk_beta_g2: [u8; 128],
    pub vk_gamme_g2: [u8; 128],
    pub vk_delta_g2: [u8; 128],
    pub vk_ic: Vec<[u8; 64]>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Completed,
    Refunded,
}

#[inline(never)]
fn verify_groth16_proof(
    verifier_registry: &VerifierRegistry,
    params: &CallbackParams,
) -> Result<()> {
    let verifying_key = verifier_registry.groth16_verifying_key();
    let public_inputs_len = verifier_registry
        .public_inputs_len()
        .ok_or_else(|| error!(ErrorCode::InvalidVerifierKey))?;

    match public_inputs_len {
        0 => verify_with_key::<0>(&verifying_key, params),
        1 => verify_with_key::<1>(&verifying_key, params),
        2 => verify_with_key::<2>(&verifying_key, params),
        3 => verify_with_key::<3>(&verifying_key, params),
        4 => verify_with_key::<4>(&verifying_key, params),
        5 => verify_with_key::<5>(&verifying_key, params),
        6 => verify_with_key::<6>(&verifying_key, params),
        7 => verify_with_key::<7>(&verifying_key, params),
        8 => verify_with_key::<8>(&verifying_key, params),
        9 => verify_with_key::<9>(&verifying_key, params),
        10 => verify_with_key::<10>(&verifying_key, params),
        11 => verify_with_key::<11>(&verifying_key, params),
        12 => verify_with_key::<12>(&verifying_key, params),
        13 => verify_with_key::<13>(&verifying_key, params),
        14 => verify_with_key::<14>(&verifying_key, params),
        15 => verify_with_key::<15>(&verifying_key, params),
        16 => verify_with_key::<16>(&verifying_key, params),
        _ => Err(error!(ErrorCode::UnsupportedVerifierPublicInputsLength)),
    }
}

#[inline(never)]
fn verify_with_key<const N: usize>(
    verifying_key: &Groth16Verifyingkey<'_>,
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

#[inline(never)]
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

#[inline(never)]
fn validate_result_size(result: &[u8]) -> Result<()> {
    require!(result.len() <= MAX_RESULT_BYTES, ErrorCode::ResultTooLarge);
    Ok(())
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
    #[cfg(not(target_os = "solana"))]
    {
        let _ = (
            callback_program,
            request_metadata,
            result_account,
            prover,
            remaining_accounts,
            request_id,
            result,
        );
        Ok(())
    }

    #[cfg(target_os = "solana")]
    {
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
    #[msg("Callback close target does not match the original payer")]
    RequestPayerMismatch,
    #[msg("Refund caller does not match the original payer")]
    RefundPayerMismatch,
    #[msg("No verifier is registered for this computation ID")]
    UnknownComputationId,
    #[msg("Stored verifier key is invalid")]
    InvalidVerifierKey,
    #[msg("Verifier key has too many public inputs (maximum 16)")]
    VerifierKeyTooManyPublicInputs,
    #[msg("Signer is not the registered authority for this verifier")]
    VerifierAuthorityMismatch,
    #[msg("Verifier public-input length is not supported by the on-chain dispatcher")]
    UnsupportedVerifierPublicInputsLength,
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

    #[test]
    fn validate_result_size_accepts_maximum_payload() {
        validate_result_size(&vec![0u8; MAX_RESULT_BYTES]).unwrap();
    }

    #[test]
    fn validate_result_size_rejects_oversized_payload() {
        let err = validate_result_size(&vec![0u8; MAX_RESULT_BYTES + 1]).unwrap_err();
        assert_eq!(err, error!(ErrorCode::ResultTooLarge));
    }

    #[test]
    fn verifier_registry_reconstructs_groth16_key() {
        let registry = VerifierRegistry {
            computation_id: DEMO_COMPUTATION_ID,
            authority: Pubkey::default(),
            vk_alpha_g1: verifier_registry::DEMO_VERIFYING_KEY.vk_alpha_g1,
            vk_beta_g2: verifier_registry::DEMO_VERIFYING_KEY.vk_beta_g2,
            vk_gamme_g2: verifier_registry::DEMO_VERIFYING_KEY.vk_gamme_g2,
            vk_delta_g2: verifier_registry::DEMO_VERIFYING_KEY.vk_delta_g2,
            vk_ic: verifier_registry::DEMO_VERIFYING_KEY.vk_ic.to_vec(),
            bump: 255,
        };

        let key = registry.groth16_verifying_key();
        assert_eq!(
            key.nr_pubinputs,
            verifier_registry::DEMO_VERIFYING_KEY.nr_pubinputs
        );
        assert_eq!(
            key.vk_alpha_g1,
            verifier_registry::DEMO_VERIFYING_KEY.vk_alpha_g1
        );
        assert_eq!(
            key.vk_beta_g2,
            verifier_registry::DEMO_VERIFYING_KEY.vk_beta_g2
        );
        assert_eq!(
            key.vk_gamme_g2,
            verifier_registry::DEMO_VERIFYING_KEY.vk_gamme_g2
        );
        assert_eq!(
            key.vk_delta_g2,
            verifier_registry::DEMO_VERIFYING_KEY.vk_delta_g2
        );
        assert_eq!(key.vk_ic, verifier_registry::DEMO_VERIFYING_KEY.vk_ic);
    }

    #[test]
    fn verifier_registry_space_matches_layout() {
        assert_eq!(
            VerifierRegistry::space_for(10),
            8 + 32 + 32 + 64 + 128 + 128 + 128 + 4 + (10 * 64) + 1
        );
    }
}
