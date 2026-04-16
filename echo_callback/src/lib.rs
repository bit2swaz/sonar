//! Echo-callback program — integration-test helper.
//!
//! Receives the `sonar_callback` CPI from the Sonar verifier program and does
//! nothing.  Its sole purpose is to give the Sonar program a real, executable
//! callback target so that the callback instruction can complete successfully
//! in TypeScript integration tests.
//!
//! Security note: this program must NEVER be deployed to mainnet.  It accepts
//! any caller without any validation and is intentionally a no-op.

// Anchor macros emit cfg-flags that only exist on the sbf target.
#![cfg_attr(not(target_os = "solana"), allow(unexpected_cfgs))]
#![allow(clippy::diverging_sub_expression)]

use anchor_lang::prelude::*;

// Placeholder ID — the TypeScript tests always read the actual deployed address
// from target/deploy/echo_callback-keypair.json, so this constant is only used
// by the IDL and by `anchor.workspace.EchoCallback` (which we do not use).
declare_id!("J7jsJVQz6xbWFhyxRbzk7nH5ALhStztUNR1nPupnyjxS");

#[program]
pub mod echo_callback {
    use super::*;

    /// Accepts the Sonar callback CPI and returns immediately.
    ///
    /// The discriminator `sha256("global:sonar_callback")[..8]` ==
    /// `[165, 188, 38, 190, 145, 138, 75, 149]`, which matches the constant
    /// `SONAR_CALLBACK_DISCRIMINATOR` hard-coded in the Sonar program.
    pub fn sonar_callback(
        _ctx: Context<SonarCallbackCtx>,
        _request_id: [u8; 32],
        _result: Vec<u8>,
    ) -> Result<()> {
        Ok(())
    }
}

/// Accounts passed by the Sonar program's `invoke_callback_program` helper.
///
/// The Sonar program sends:
///   AccountMeta::new_readonly(request_metadata.key, false)  — non-signer, read-only
///   AccountMeta::new_readonly(result_account.key,   false)  — non-signer, read-only
///   AccountMeta::new_readonly(prover.key,           true)   — signer,     read-only
#[derive(Accounts)]
pub struct SonarCallbackCtx<'info> {
    /// CHECK: Read-only metadata passed by Sonar — no validation required.
    pub request_metadata: UncheckedAccount<'info>,
    /// CHECK: Read-only result account passed by Sonar — no validation required.
    pub result_account: UncheckedAccount<'info>,
    /// The prover that submitted the callback (signer).
    pub prover: Signer<'info>,
}
