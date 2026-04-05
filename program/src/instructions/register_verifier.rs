use anchor_lang::prelude::*;

use crate::{RegisterVerifier, RegisterVerifierParams};

pub fn handler(ctx: Context<RegisterVerifier>, params: RegisterVerifierParams) -> Result<()> {
    let verifier_registry = &mut ctx.accounts.verifier_registry;
    verifier_registry.computation_id = params.computation_id;
    verifier_registry.authority = ctx.accounts.authority.key();
    verifier_registry.vkey = params.vkey;
    verifier_registry.bump = ctx.bumps.verifier_registry;
    Ok(())
}
