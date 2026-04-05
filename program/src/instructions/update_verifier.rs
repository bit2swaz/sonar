use anchor_lang::prelude::*;

use crate::{RegisterVerifierParams, UpdateVerifier};

/// Maximum supported vk_ic length (mirrored from register_verifier).
const MAX_VK_IC_LEN: usize = 17;

pub fn handler(ctx: Context<UpdateVerifier>, params: RegisterVerifierParams) -> Result<()> {
    require!(
        !params.vk_ic.is_empty(),
        crate::ErrorCode::InvalidVerifierKey
    );
    require!(
        params.vk_ic.len() <= MAX_VK_IC_LEN,
        crate::ErrorCode::VerifierKeyTooManyPublicInputs
    );

    let verifier_registry = &mut ctx.accounts.verifier_registry;
    verifier_registry.vk_alpha_g1 = params.vk_alpha_g1;
    verifier_registry.vk_beta_g2 = params.vk_beta_g2;
    verifier_registry.vk_gamme_g2 = params.vk_gamme_g2;
    verifier_registry.vk_delta_g2 = params.vk_delta_g2;
    verifier_registry.vk_ic = params.vk_ic;
    Ok(())
}
