use anyhow::{ensure, Context};
use num_bigint::BigUint;
use sp1_sdk::{SP1Proof, SP1ProofWithPublicValues};

use crate::sp1_wrapper::{deserialize_proof, load_proof_bundle};

const SP1_ENCODED_GROTH16_PREFIX_BYTES: usize = 96;
const SONAR_GROTH16_PROOF_BYTES: usize = 256;
const SONAR_GROTH16_PUBLIC_INPUT_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Groth16Payload {
    pub proof: Vec<u8>,
    pub public_inputs: Vec<Vec<u8>>,
}

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

pub fn extract_sp1_groth16_payload(stark_proof: &[u8]) -> anyhow::Result<Groth16Payload> {
    let bundle = load_proof_bundle(stark_proof)?;
    let groth16_proof = deserialize_proof(&bundle.groth16_proof)
        .context("failed to deserialize cached Groth16 proof")?;

    extract_sp1_groth16_payload_from_proof(&groth16_proof)
}

pub(crate) fn extract_sp1_groth16_payload_from_proof(
    proof_with_public_values: &SP1ProofWithPublicValues,
) -> anyhow::Result<Groth16Payload> {
    let SP1Proof::Groth16(groth16_proof) = &proof_with_public_values.proof else {
        anyhow::bail!("cached SP1 proof is not a Groth16 proof");
    };

    let proof =
        sp1_proof_to_raw_groth16_bytes(&groth16_proof.raw_proof, &groth16_proof.encoded_proof)?;
    let public_inputs = groth16_proof
        .public_inputs
        .iter()
        .map(|value| decimal_public_input_to_bytes32(value))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Groth16Payload {
        proof,
        public_inputs,
    })
}

fn sp1_proof_to_raw_groth16_bytes(raw_proof: &str, encoded_proof: &str) -> anyhow::Result<Vec<u8>> {
    if !raw_proof.is_empty() {
        return raw_proof_to_raw_groth16_bytes(raw_proof);
    }

    encoded_proof_to_raw_groth16_bytes(encoded_proof)
}

fn raw_proof_to_raw_groth16_bytes(raw_proof: &str) -> anyhow::Result<Vec<u8>> {
    ensure!(
        !raw_proof.is_empty(),
        "SP1 mock Groth16 proofs do not contain raw proof bytes; on-chain Sonar callbacks require real Groth16 proofs"
    );

    let proof = hex::decode(raw_proof).context("failed to decode SP1 Groth16 raw proof")?;
    ensure!(
        proof.len() == SONAR_GROTH16_PROOF_BYTES,
        "decoded Sonar Groth16 proof has invalid length: expected {}, got {}",
        SONAR_GROTH16_PROOF_BYTES,
        proof.len()
    );

    Ok(proof)
}

fn encoded_proof_to_raw_groth16_bytes(encoded_proof: &str) -> anyhow::Result<Vec<u8>> {
    ensure!(
        !encoded_proof.is_empty(),
        "SP1 mock Groth16 proofs do not contain raw proof bytes; on-chain Sonar callbacks require real Groth16 proofs"
    );

    let encoded =
        hex::decode(encoded_proof).context("failed to decode SP1 Groth16 encoded proof")?;
    ensure!(
        encoded.len() >= SP1_ENCODED_GROTH16_PREFIX_BYTES + SONAR_GROTH16_PROOF_BYTES,
        "encoded SP1 Groth16 proof is too short: expected at least {}, got {}",
        SP1_ENCODED_GROTH16_PREFIX_BYTES + SONAR_GROTH16_PROOF_BYTES,
        encoded.len()
    );

    let proof = encoded[SP1_ENCODED_GROTH16_PREFIX_BYTES..].to_vec();
    ensure!(
        proof.len() == SONAR_GROTH16_PROOF_BYTES,
        "decoded Sonar Groth16 proof has invalid length: expected {}, got {}",
        SONAR_GROTH16_PROOF_BYTES,
        proof.len()
    );

    Ok(proof)
}

fn decimal_public_input_to_bytes32(value: &str) -> anyhow::Result<Vec<u8>> {
    let parsed = BigUint::parse_bytes(value.as_bytes(), 10).context(format!(
        "failed to parse SP1 Groth16 public input '{value}'"
    ))?;
    let bytes = parsed.to_bytes_be();

    ensure!(
        bytes.len() <= SONAR_GROTH16_PUBLIC_INPUT_BYTES,
        "SP1 Groth16 public input exceeds 32 bytes: {value}"
    );

    let mut padded = vec![0u8; SONAR_GROTH16_PUBLIC_INPUT_BYTES];
    let start = SONAR_GROTH16_PUBLIC_INPUT_BYTES.saturating_sub(bytes.len());
    padded[start..].copy_from_slice(&bytes);
    Ok(padded)
}

#[cfg(test)]
mod tests {
    use sp1_sdk::{SP1Proof, SP1ProofWithPublicValues, SP1PublicValues};
    use sp1_verifier::Groth16Bn254Proof;

    use crate::sp1_wrapper::Sp1ProofBundle;

    use super::{
        decimal_public_input_to_bytes32, encoded_proof_to_raw_groth16_bytes,
        extract_sp1_groth16_payload, extract_sp1_groth16_payload_from_proof,
        raw_proof_to_raw_groth16_bytes, sp1_proof_to_raw_groth16_bytes,
    };

    fn synthetic_groth16_proof() -> (SP1ProofWithPublicValues, Vec<u8>, Vec<u8>) {
        let raw_proof = (0..256u16).map(|value| value as u8).collect::<Vec<_>>();
        let public_values = SP1PublicValues::from(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let public_values_hash = public_values.hash_bn254().to_string();
        let expected_hash_bytes =
            decimal_public_input_to_bytes32(&public_values_hash).expect("hash should fit");

        let proof = SP1ProofWithPublicValues::new(
            SP1Proof::Groth16(Groth16Bn254Proof {
                public_inputs: [
                    "1".to_string(),
                    public_values_hash,
                    "2".to_string(),
                    "3".to_string(),
                    "4".to_string(),
                ],
                encoded_proof: String::new(),
                raw_proof: hex::encode(&raw_proof),
                groth16_vkey_hash: [7u8; 32],
            }),
            public_values,
            "test".to_string(),
        );

        (proof, raw_proof, expected_hash_bytes)
    }

    #[test]
    fn decimal_public_input_to_bytes32_left_pads_values() {
        let parsed = decimal_public_input_to_bytes32("4660").expect("decimal parsing should work");

        assert_eq!(parsed.len(), 32);
        assert_eq!(&parsed[30..], &[0x12, 0x34]);
        assert!(parsed[..30].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn encoded_proof_to_raw_groth16_bytes_strips_sp1_prefix() {
        let encoded = (0..352u16).map(|value| value as u8).collect::<Vec<_>>();
        let proof = encoded_proof_to_raw_groth16_bytes(&hex::encode(&encoded))
            .expect("encoded proof should decode");

        assert_eq!(proof.len(), 256);
        assert_eq!(proof, encoded[96..].to_vec());
    }

    #[test]
    fn raw_proof_to_raw_groth16_bytes_decodes_hex_payload() {
        let raw = (0..256u16).map(|value| value as u8).collect::<Vec<_>>();
        let proof =
            raw_proof_to_raw_groth16_bytes(&hex::encode(&raw)).expect("raw proof should decode");

        assert_eq!(proof, raw);
    }

    #[test]
    fn sp1_proof_to_raw_groth16_bytes_prefers_raw_proof() {
        let raw = vec![0xAB; 256];
        let encoded = (0..352u16).map(|value| value as u8).collect::<Vec<_>>();
        let proof = sp1_proof_to_raw_groth16_bytes(&hex::encode(&raw), &hex::encode(&encoded))
            .expect("raw proof should take precedence over encoded proof");

        assert_eq!(proof, raw);
    }

    #[test]
    fn extract_sp1_groth16_payload_from_proof_preserves_shape() {
        let (proof, raw_proof, expected_hash_bytes) = synthetic_groth16_proof();

        let payload = extract_sp1_groth16_payload_from_proof(&proof)
            .expect("direct proof extraction should work");

        assert_eq!(payload.proof, raw_proof);
        assert_eq!(payload.public_inputs.len(), 5);
        assert!(payload.public_inputs.iter().all(|input| input.len() == 32));
        assert_eq!(payload.public_inputs[1], expected_hash_bytes);
    }

    #[test]
    fn extract_sp1_groth16_payload_matches_direct_and_bundle_paths() {
        let (proof, raw_proof, expected_hash_bytes) = synthetic_groth16_proof();
        let bundle = Sp1ProofBundle {
            public_values: proof.public_values.as_slice().to_vec(),
            stark_proof: Vec::new(),
            groth16_proof: bincode::serialize(&proof).expect("proof should serialize"),
        };
        let serialized_bundle = bincode::serialize(&bundle).expect("bundle should serialize");

        let direct = extract_sp1_groth16_payload_from_proof(&proof)
            .expect("direct proof extraction should work");
        let from_bundle =
            extract_sp1_groth16_payload(&serialized_bundle).expect("bundle extraction should work");

        assert_eq!(from_bundle, direct);
        assert_eq!(from_bundle.proof, raw_proof);
        assert_eq!(from_bundle.public_inputs[1], expected_hash_bytes);
    }
}
