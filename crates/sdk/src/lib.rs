#![cfg_attr(not(target_os = "solana"), allow(unexpected_cfgs))]

use anchor_lang::{
    prelude::*, solana_program::instruction::AccountMeta, ToAccountInfos, ToAccountMetas,
};

pub mod r#macro;

pub use sonar_program::{
    program::Sonar, RequestParams, DEMO_COMPUTATION_ID, HISTORICAL_AVG_COMPUTATION_ID,
};

const REQUEST_METADATA_SEED: &[u8] = b"request";
const RESULT_ACCOUNT_SEED: &[u8] = b"result";

/// Accounts required to submit a Sonar request via CPI.
///
/// This bundle is designed to be wrapped in [`CpiContext`] and passed to
/// [`request`]. Unlike the raw Anchor-generated CPI accounts, it also carries
/// the caller-chosen `request_id`, which the SDK uses to derive and validate
/// the Sonar PDAs before dispatching the CPI.
///
/// The `request_metadata` and `result_account` fields should be the writable,
/// uninitialized Sonar PDAs for this request. The helper verifies both account
/// addresses against `request_id` and the Sonar program id in the supplied
/// [`CpiContext`].
#[derive(Clone)]
pub struct Request<'info> {
    /// Unique nonce for this request.
    pub request_id: [u8; 32],
    /// Fee payer that funds the request escrow and PDA creation.
    pub payer: AccountInfo<'info>,
    /// Program that Sonar will call during callback execution.
    pub callback_program: AccountInfo<'info>,
    /// Writable Sonar request metadata PDA derived from `request_id`.
    pub request_metadata: AccountInfo<'info>,
    /// Writable Sonar result PDA derived from `request_id`.
    pub result_account: AccountInfo<'info>,
    /// System program used by Sonar to create PDA accounts.
    pub system_program: AccountInfo<'info>,
}

impl<'info> ToAccountInfos<'info> for Request<'info> {
    fn to_account_infos(&self) -> Vec<AccountInfo<'info>> {
        vec![
            self.payer.clone(),
            self.callback_program.clone(),
            self.request_metadata.clone(),
            self.result_account.clone(),
            self.system_program.clone(),
        ]
    }
}

impl ToAccountMetas for Request<'_> {
    fn to_account_metas(&self, _is_signer: Option<bool>) -> Vec<AccountMeta> {
        vec![
            AccountMeta {
                pubkey: *self.payer.key,
                is_signer: self.payer.is_signer,
                is_writable: self.payer.is_writable,
            },
            AccountMeta {
                pubkey: *self.callback_program.key,
                is_signer: self.callback_program.is_signer,
                is_writable: self.callback_program.is_writable,
            },
            AccountMeta {
                pubkey: *self.request_metadata.key,
                is_signer: self.request_metadata.is_signer,
                is_writable: self.request_metadata.is_writable,
            },
            AccountMeta {
                pubkey: *self.result_account.key,
                is_signer: self.result_account.is_signer,
                is_writable: self.result_account.is_writable,
            },
            AccountMeta {
                pubkey: *self.system_program.key,
                is_signer: self.system_program.is_signer,
                is_writable: self.system_program.is_writable,
            },
        ]
    }
}

/// Submit a Sonar computation request from another Anchor program.
///
/// This helper wraps the raw `sonar_program::cpi::request` call and removes the
/// repetitive parts every downstream program would otherwise need to write by
/// hand:
///
/// - it accepts a single ergonomic [`Request`] bundle
/// - it derives the expected `request_metadata` and `result_account` PDAs from
///   the caller-provided `request_id`
/// - it validates that the supplied writable accounts match those derived PDAs
/// - it forwards signer seeds and remaining accounts from the provided
///   [`CpiContext`], allowing downstream programs to register callback-side
///   account metas that the coordinator can replay later
///
/// The caller is responsible for choosing a unique `request_id` nonce and for
/// passing the matching writable Sonar PDA accounts in the context. A common
/// pattern is to derive or hash the nonce inside the caller program, then reuse
/// it both in the local request state and in the [`Request`] bundle passed here.
///
/// # Example
///
/// ```ignore
/// use anchor_lang::prelude::*;
/// use sonar_sdk::{self, Sonar, HISTORICAL_AVG_COMPUTATION_ID};
///
/// #[derive(Accounts)]
/// pub struct RequestHistoricalAvg<'info> {
///     #[account(mut)]
///     pub payer: Signer<'info>,
///     #[account(address = crate::ID)]
///     /// CHECK: This program is recorded by Sonar and called back later.
///     pub callback_program: UncheckedAccount<'info>,
///     #[account(mut)]
///     /// CHECK: Must be the Sonar request PDA for `request_id`.
///     pub request_metadata: UncheckedAccount<'info>,
///     #[account(mut)]
///     /// CHECK: Must be the Sonar result PDA for `request_id`.
///     pub result_account: UncheckedAccount<'info>,
///     pub sonar_program: Program<'info, Sonar>,
///     pub system_program: Program<'info, System>,
/// }
///
/// pub fn request_historical_avg(
///     ctx: Context<RequestHistoricalAvg>,
///     request_id: [u8; 32],
///     inputs: Vec<u8>,
///     deadline: u64,
///     fee: u64,
/// ) -> Result<()> {
///     let cpi_ctx = CpiContext::new(
///         ctx.accounts.sonar_program.to_account_info(),
///         sonar_sdk::Request {
///             request_id,
///             payer: ctx.accounts.payer.to_account_info(),
///             callback_program: ctx.accounts.callback_program.to_account_info(),
///             request_metadata: ctx.accounts.request_metadata.to_account_info(),
///             result_account: ctx.accounts.result_account.to_account_info(),
///             system_program: ctx.accounts.system_program.to_account_info(),
///         },
///     );
///
///     sonar_sdk::request(
///         cpi_ctx,
///         HISTORICAL_AVG_COMPUTATION_ID,
///         inputs,
///         deadline,
///         fee,
///     )
/// }
/// ```
pub fn request<'info>(
    ctx: CpiContext<'_, '_, '_, 'info, Request<'info>>,
    computation_id: [u8; 32],
    inputs: Vec<u8>,
    deadline: u64,
    fee: u64,
) -> Result<()> {
    let request_id = ctx.accounts.request_id;
    let sonar_program_id = ctx.program.key();

    let (expected_request_metadata, _request_metadata_bump) = Pubkey::find_program_address(
        &[REQUEST_METADATA_SEED, request_id.as_ref()],
        &sonar_program_id,
    );
    let (expected_result_account, _result_account_bump) = Pubkey::find_program_address(
        &[RESULT_ACCOUNT_SEED, request_id.as_ref()],
        &sonar_program_id,
    );

    require_keys_eq!(
        ctx.accounts.request_metadata.key(),
        expected_request_metadata,
        SonarSdkError::InvalidRequestMetadataPda
    );
    require_keys_eq!(
        ctx.accounts.result_account.key(),
        expected_result_account,
        SonarSdkError::InvalidResultAccountPda
    );

    let cpi_accounts = sonar_program::cpi::accounts::Request {
        payer: ctx.accounts.payer.clone(),
        callback_program: ctx.accounts.callback_program.clone(),
        request_metadata: ctx.accounts.request_metadata.clone(),
        result_account: ctx.accounts.result_account.clone(),
        system_program: ctx.accounts.system_program.clone(),
    };
    let cpi_ctx = CpiContext::new_with_signer(ctx.program, cpi_accounts, ctx.signer_seeds)
        .with_remaining_accounts(ctx.remaining_accounts.to_vec());

    sonar_program::cpi::request(
        cpi_ctx,
        RequestParams {
            request_id,
            computation_id,
            inputs,
            deadline,
            fee,
        },
    )
}

#[error_code]
pub enum SonarSdkError {
    #[msg("request_metadata does not match the Sonar PDA derived from request_id")]
    InvalidRequestMetadataPda,
    #[msg("result_account does not match the Sonar PDA derived from request_id")]
    InvalidResultAccountPda,
}
