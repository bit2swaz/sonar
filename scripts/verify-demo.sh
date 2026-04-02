#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_SCRIPT="${ROOT_DIR}/scripts/demo-historical-avg.sh"
STATE_JSON="${ROOT_DIR}/.demo/historical-avg/state.json"
OUTPUT_FILE="$(mktemp)"

: "${POSTGRES_PORT:=15432}"
: "${REDIS_PORT:=16379}"
: "${INDEXER_HTTP_PORT:=18080}"
: "${RPC_PORT:=18899}"
: "${FAUCET_PORT:=19900}"
: "${DYNAMIC_PORT_START:=20000}"
: "${DYNAMIC_PORT_END:=20030}"

export POSTGRES_PORT
export REDIS_PORT
export INDEXER_HTTP_PORT
export RPC_PORT
export FAUCET_PORT
export DYNAMIC_PORT_START
export DYNAMIC_PORT_END

cleanup() {
	rm -f "${OUTPUT_FILE}"
}

trap cleanup EXIT

need_cmd() {
	command -v "$1" >/dev/null 2>&1 || {
		echo "missing required command: $1" >&2
		exit 1
	}
}

need_cmd python3

echo "[verify-demo] using ports: postgres=${POSTGRES_PORT} redis=${REDIS_PORT} indexer=${INDEXER_HTTP_PORT} rpc=${RPC_PORT} faucet=${FAUCET_PORT} dynamic=${DYNAMIC_PORT_START}-${DYNAMIC_PORT_END}"

if ! "${DEMO_SCRIPT}" --no-pause demo | tee "${OUTPUT_FILE}"; then
	echo "VERIFICATION FAILED: demo command exited non-zero" >&2
	exit 1
fi

historical_avg_result="$(grep -E '^historical_avg_result=' "${OUTPUT_FILE}" | tail -n 1 | cut -d= -f2-)"
expected_avg="$(grep -E '^expected_avg=' "${OUTPUT_FILE}" | tail -n 1 | cut -d= -f2-)"

if [[ -z "${historical_avg_result}" || -z "${expected_avg}" ]]; then
	echo "VERIFICATION FAILED: could not parse result output" >&2
	exit 1
fi

python3 - "${STATE_JSON}" <<'PY'
import json
import sys

with open(sys.argv[1], 'r', encoding='utf-8') as fh:
	state = json.load(fh)

required = ['request_metadata', 'result_account', 'request_id_hex', 'expected_avg']
missing = [key for key in required if not state.get(key)]
if missing:
	raise SystemExit(f"missing demo state fields: {', '.join(missing)}")
PY

if [[ "${historical_avg_result}" != "${expected_avg}" ]]; then
	echo "VERIFICATION FAILED: historical_avg_result=${historical_avg_result} expected_avg=${expected_avg}" >&2
	exit 1
fi

echo "VERIFICATION PASSED"
echo "historical_avg_result=${historical_avg_result}"
echo "expected_avg=${expected_avg}"