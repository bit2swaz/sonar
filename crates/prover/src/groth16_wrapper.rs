use anyhow::{ensure, Context};

use crate::sp1_wrapper::{deserialize_proof, load_proof_bundle};

pub fn wrap_stark_to_groth16(
    stark_proof: &[u8],
    public_inputs: &[Vec<u8>],
) -> anyhow::Result<Vec<u8>> {
    ensure!(
        !public_inputs.is_empty(),
        "public inputs are required for Groth16 wrapping"
    );

    let bundle = load_proof_bundle(stark_proof)?;
    let groth16_proof = deserialize_proof(&bundle.groth16_proof)
        .context("failed to deserialize cached Groth16 proof")?;

    let encoded = groth16_proof.bytes();
    if !encoded.is_empty() {
        return Ok(encoded);
    }

    Ok(bundle.groth16_proof)
}
