#![cfg_attr(not(target_os = "solana"), allow(unexpected_cfgs))]
#![allow(clippy::diverging_sub_expression)]

use anchor_lang::prelude::*;
use sonar_program::{self, program::Sonar, RequestParams as SonarRequestParams};

declare_id!("2GNQ6iMwsH5RLJQ5Fhj5ieHmMhzgCT3y7DKx8tvspqJd");

const RAW_HISTORICAL_AVG_INPUTS_LEN: usize = 48;
const MAX_RESULT_BYTES: usize = 8;

#[program]
pub mod historical_avg_client {
    use super::*;

    pub fn request_historical_avg(
        ctx: Context<RequestHistoricalAvg>,
        params: HistoricalAvgRequestParams,
    ) -> Result<()> {
        let callback_state = &mut ctx.accounts.callback_state;
        callback_state.request_id = params.request_id;
        callback_state.target_account = params.account;
        callback_state.from_slot = params.from_slot;
        callback_state.to_slot = params.to_slot;
        callback_state.result = Vec::new();
        callback_state.is_set = false;
        callback_state.bump = ctx.bumps.callback_state;

        let mut raw_inputs = Vec::with_capacity(RAW_HISTORICAL_AVG_INPUTS_LEN);
        raw_inputs.extend_from_slice(params.account.as_ref());
        raw_inputs.extend_from_slice(&params.from_slot.to_le_bytes());
        raw_inputs.extend_from_slice(&params.to_slot.to_le_bytes());

        let cpi_accounts = sonar_program::cpi::accounts::Request {
            payer: ctx.accounts.payer.to_account_info(),
            callback_program: ctx.accounts.callback_program.to_account_info(),
            request_metadata: ctx.accounts.request_metadata.to_account_info(),
            result_account: ctx.accounts.result_account.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.sonar_program.to_account_info(), cpi_accounts);

        sonar_program::cpi::request(
            cpi_ctx,
            SonarRequestParams {
                request_id: params.request_id,
                computation_id: sonar_program::HISTORICAL_AVG_COMPUTATION_ID,
                inputs: raw_inputs,
                deadline: params.deadline,
                fee: params.fee,
            },
        )
    }

    pub fn sonar_callback(
        ctx: Context<SonarCallback>,
        request_id: [u8; 32],
        result: Vec<u8>,
    ) -> Result<()> {
        require!(result.len() <= MAX_RESULT_BYTES, ErrorCode::ResultTooLarge);

        let callback_state = &mut ctx.accounts.callback_state;
        require!(
            callback_state.request_id == request_id,
            ErrorCode::RequestIdMismatch
        );

        callback_state.result = result;
        callback_state.is_set = true;
        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct HistoricalAvgRequestParams {
    pub request_id: [u8; 32],
    pub account: [u8; 32],
    pub from_slot: u64,
    pub to_slot: u64,
    pub fee: u64,
    pub deadline: u64,
}

#[derive(Accounts)]
#[instruction(params: HistoricalAvgRequestParams)]
pub struct RequestHistoricalAvg<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(address = crate::ID)]
    /// CHECK: This program account is recorded in Sonar request metadata and later invoked as the callback target.
    pub callback_program: UncheckedAccount<'info>,
    #[account(
        init,
        payer = payer,
        space = CallbackState::LEN,
        seeds = [b"client-result", params.request_id.as_ref()],
        bump,
    )]
    pub callback_state: Account<'info, CallbackState>,
    #[account(mut)]
    /// CHECK: Initialized by the downstream Sonar CPI.
    pub request_metadata: UncheckedAccount<'info>,
    #[account(mut)]
    /// CHECK: Initialized by the downstream Sonar CPI.
    pub result_account: UncheckedAccount<'info>,
    pub sonar_program: Program<'info, Sonar>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(request_id: [u8; 32], _result: Vec<u8>)]
pub struct SonarCallback<'info> {
    /// CHECK: Read-only request metadata account supplied by Sonar.
    pub request_metadata: UncheckedAccount<'info>,
    /// CHECK: Read-only result account supplied by Sonar.
    pub result_account: UncheckedAccount<'info>,
    pub prover: Signer<'info>,
    #[account(
        mut,
        seeds = [b"client-result", request_id.as_ref()],
        bump = callback_state.bump,
    )]
    pub callback_state: Account<'info, CallbackState>,
}

#[account]
pub struct CallbackState {
    pub request_id: [u8; 32],
    pub target_account: [u8; 32],
    pub from_slot: u64,
    pub to_slot: u64,
    pub result: Vec<u8>,
    pub is_set: bool,
    pub bump: u8,
}

impl CallbackState {
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 4 + MAX_RESULT_BYTES + 1 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Callback request id does not match stored request state")]
    RequestIdMismatch,
    #[msg("Historical-average callback result is too large")]
    ResultTooLarge,
}
