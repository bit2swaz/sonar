use anyhow::Context;

use crate::{historical_avg_computation_id, sp1_wrapper::compute_historical_avg_result};

pub const HISTORICAL_AVG_CALLBACK_FIXTURE_ENV: &str =
    "SONAR_USE_HISTORICAL_AVG_CALLBACK_FIXTURE";

pub(crate) fn maybe_fixture_callback_payload(
    computation_id: &[u8; 32],
    inputs: &[u8],
) -> anyhow::Result<Option<(Vec<u8>, Vec<u8>, Vec<Vec<u8>>)>> {
    if !historical_avg_callback_fixture_enabled() || !using_mock_prover() {
        return Ok(None);
    }

    let historical_avg_id = historical_avg_computation_id()
        .context("derive historical_avg computation id for callback fixture")?;
    if computation_id != &historical_avg_id {
        return Ok(None);
    }

    let balances: Vec<u64> = bincode::deserialize(inputs)
        .context("failed to deserialize historical_avg inputs as Vec<u64>")?;
    let result = compute_historical_avg_result(&balances).to_le_bytes().to_vec();
    let public_inputs = HISTORICAL_AVG_CALLBACK_FIXTURE_PUBLIC_INPUTS
        .iter()
        .map(|input| input.to_vec())
        .collect();

    Ok(Some((
        HISTORICAL_AVG_CALLBACK_FIXTURE_PROOF.to_vec(),
        result,
        public_inputs,
    )))
}

fn historical_avg_callback_fixture_enabled() -> bool {
    std::env::var(HISTORICAL_AVG_CALLBACK_FIXTURE_ENV)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn using_mock_prover() -> bool {
    std::env::var("SP1_PROVER")
        .map(|value| value.eq_ignore_ascii_case("mock"))
        .unwrap_or(false)
}

const HISTORICAL_AVG_CALLBACK_FIXTURE_PROOF: [u8; 256] = [
    45, 206, 255, 166, 152, 55, 128, 138, 79, 217, 145, 164, 25, 74, 120, 234, 234, 217, 68,
    149, 162, 44, 133, 120, 184, 205, 12, 44, 175, 98, 168, 172, 28, 75, 118, 99, 15, 130, 53,
    222, 36, 99, 235, 81, 5, 165, 98, 197, 197, 182, 144, 40, 212, 105, 169, 142, 72, 96, 177,
    156, 174, 43, 59, 243, 40, 57, 233, 205, 180, 46, 35, 111, 215, 5, 23, 93, 12, 71, 118,
    225, 7, 46, 247, 147, 47, 130, 106, 189, 184, 80, 146, 103, 141, 52, 242, 25, 0, 203, 124,
    176, 110, 34, 151, 212, 66, 180, 238, 151, 236, 189, 133, 209, 17, 137, 205, 183, 168,
    196, 92, 159, 75, 174, 81, 168, 18, 86, 176, 56, 16, 26, 210, 20, 18, 81, 122, 142, 104,
    62, 251, 169, 98, 141, 21, 253, 50, 130, 182, 15, 33, 109, 228, 31, 79, 183, 88, 147, 174,
    108, 4, 22, 14, 129, 168, 6, 80, 246, 254, 100, 218, 131, 94, 49, 247, 211, 3, 245, 22,
    200, 177, 91, 60, 144, 147, 174, 90, 17, 19, 189, 62, 147, 152, 18, 41, 139, 183, 208,
    246, 198, 118, 127, 89, 160, 9, 27, 61, 26, 123, 180, 221, 108, 17, 166, 47, 115, 82, 48,
    132, 139, 253, 65, 152, 92, 209, 53, 37, 25, 83, 61, 252, 42, 181, 243, 16, 21, 2, 199,
    123, 96, 218, 151, 253, 86, 69, 181, 202, 109, 64, 129, 124, 254, 192, 25, 177, 199, 26,
    50,
];

const HISTORICAL_AVG_CALLBACK_FIXTURE_PUBLIC_INPUTS: [[u8; 32]; 9] = [
    [
        34, 238, 251, 182, 234, 248, 214, 189, 46, 67, 42, 25, 71, 58, 145, 58, 61, 28, 116,
        110, 60, 17, 82, 149, 178, 187, 160, 211, 37, 226, 174, 231,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        51, 152, 17, 147,
    ],
    [
        4, 247, 199, 87, 230, 85, 103, 90, 28, 183, 95, 100, 200, 46, 3, 158, 247, 196, 173,
        146, 207, 167, 108, 33, 199, 18, 13, 204, 198, 101, 223, 186,
    ],
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        7, 49, 65, 41,
    ],
    [
        7, 130, 55, 65, 197, 232, 175, 217, 44, 151, 149, 225, 75, 86, 158, 105, 43, 229, 65,
        87, 51, 150, 168, 243, 176, 175, 11, 203, 180, 149, 72, 103,
    ],
    [
        46, 93, 177, 62, 42, 66, 223, 153, 51, 193, 146, 49, 154, 41, 69, 198, 224, 13, 87,
        80, 222, 171, 37, 141, 0, 1, 50, 172, 18, 28, 213, 213,
    ],
    [
        40, 141, 45, 3, 180, 200, 250, 112, 108, 94, 35, 143, 82, 63, 125, 9, 147, 37, 191,
        75, 62, 221, 138, 20, 166, 151, 219, 237, 254, 58, 230, 189,
    ],
    [
        33, 100, 143, 241, 11, 251, 73, 141, 229, 57, 129, 168, 83, 23, 235, 147, 138, 225,
        177, 250, 13, 97, 226, 162, 6, 232, 52, 95, 128, 84, 90, 202,
    ],
    [
        25, 178, 1, 208, 219, 169, 222, 123, 113, 202, 165, 77, 183, 98, 103, 237, 187, 93,
        178, 95, 169, 156, 38, 100, 125, 218, 104, 94, 104, 119, 13, 21,
    ],
];