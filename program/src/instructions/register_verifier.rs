use anchor_lang::prelude::*;

use crate::{RegisterVerifier, RegisterVerifierParams};

/// Maximum supported vk_ic length: one generator point plus one point per public input.
/// `verify_groth16_proof` supports up to 16 public inputs, so vk_ic must be 1..=17.
const MAX_VK_IC_LEN: usize = 17;

pub fn handler(ctx: Context<RegisterVerifier>, params: RegisterVerifierParams) -> Result<()> {
    require!(
        !params.vk_ic.is_empty(),
        crate::ErrorCode::InvalidVerifierKey
    );
    require!(
        params.vk_ic.len() <= MAX_VK_IC_LEN,
        crate::ErrorCode::VerifierKeyTooManyPublicInputs
    );

    let verifier_registry = &mut ctx.accounts.verifier_registry;
    verifier_registry.computation_id = params.computation_id;
    verifier_registry.authority = ctx.accounts.authority.key();
    verifier_registry.vk_alpha_g1 = params.vk_alpha_g1;
    verifier_registry.vk_beta_g2 = params.vk_beta_g2;
    verifier_registry.vk_gamme_g2 = params.vk_gamme_g2;
    verifier_registry.vk_delta_g2 = params.vk_delta_g2;
    verifier_registry.vk_ic = params.vk_ic;
    verifier_registry.bump = ctx.bumps.verifier_registry;
    Ok(())
}
