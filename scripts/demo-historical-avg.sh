#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

export SP1_PROVER="${SP1_PROVER:-mock}"

echo "running local historical-average end-to-end demo"
echo "using SP1_PROVER=${SP1_PROVER}"

cargo test --test e2e_historical_avg -- --nocapture
