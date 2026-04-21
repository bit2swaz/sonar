#!/usr/bin/env bash
set -Eeuo pipefail

log() {
  printf '[devnet-smoke-bench] %s\n' "$*"
}

die() {
  log "ERROR: $*"
  exit 1
}

on_error() {
  local exit_code=$?
  local line_number=${1:-unknown}
  log "ERROR: command failed at line ${line_number}: ${BASH_COMMAND}"
  print_log_tails || true
  exit "$exit_code"
}

trap 'on_error $LINENO' ERR

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANCHOR_TOML="$ROOT_DIR/Anchor.toml"
DEFAULT_RPC_URL="https://solana-devnet.core.chainstack.com/51a4443c8b33222e5327f331e007ec91"
DEFAULT_WS_URL="wss://solana-devnet.core.chainstack.com/51a4443c8b33222e5327f331e007ec91"
DEFAULT_CONFIG_PATH="$ROOT_DIR/config/devnet-smoke.toml"
DEFAULT_REDIS_PORT=16379
DEFAULT_FIB_N=30
DEFAULT_LOAD_REQUESTS=10
DEFAULT_THROUGHPUT_TPS=2
DEFAULT_THROUGHPUT_DURATION=30
DEFAULT_TIMEOUT_SECONDS=900
DEFAULT_POLL_INTERVAL_MS=2000
DEFAULT_PROGRAM_ID="Gf7RSZYmfNJ5kv2AJvcv5rjCANP6ePExJR19D91MECLY"
DEFAULT_CALLBACK_PROGRAM_ID="J7jsJVQz6xbWFhyxRbzk7nH5ALhStztUNR1nPupnyjxS"
DEFAULT_SERVICE_PROFILE="release"
DEFAULT_ESTIMATED_REQUEST_COST_LAMPORTS=75000000
DEFAULT_BENCHMARK_BALANCE_CUSHION_LAMPORTS=500000000
REDIS_CONTAINER_NAME="sonar-devnet-smoke-redis"
NO_DNA="${NO_DNA:-1}"

RUN_DEPLOY=1
RUN_REGISTER=1
RUN_CU=1
RUN_COLD_WARM=1
RUN_LOAD=1
RUN_THROUGHPUT=1
KEEP_SERVICES=0
SKIP_BUILD=0
DRY_RUN=0
FORCE_DEPLOY=0
ALLOW_LOW_BALANCE="${SONAR_ALLOW_LOW_BENCHMARK_BALANCE:-0}"

RPC_URL="${SOLANA_RPC_URL:-$DEFAULT_RPC_URL}"
WS_URL="${SOLANA_WS_URL:-$DEFAULT_WS_URL}"
REDIS_PORT="${REDIS_PORT:-$DEFAULT_REDIS_PORT}"
CONFIG_PATH="${SONAR_DEVNET_SMOKE_CONFIG:-$DEFAULT_CONFIG_PATH}"
SERVICE_PROFILE="${SONAR_DEVNET_SMOKE_SERVICE_PROFILE:-$DEFAULT_SERVICE_PROFILE}"
ESTIMATED_REQUEST_COST_LAMPORTS="${SONAR_BENCHMARK_ESTIMATED_REQUEST_COST_LAMPORTS:-$DEFAULT_ESTIMATED_REQUEST_COST_LAMPORTS}"
BENCHMARK_BALANCE_CUSHION_LAMPORTS="${SONAR_BENCHMARK_BALANCE_CUSHION_LAMPORTS:-$DEFAULT_BENCHMARK_BALANCE_CUSHION_LAMPORTS}"
FIB_N="$DEFAULT_FIB_N"
LOAD_REQUESTS="$DEFAULT_LOAD_REQUESTS"
THROUGHPUT_TPS="$DEFAULT_THROUGHPUT_TPS"
THROUGHPUT_DURATION="$DEFAULT_THROUGHPUT_DURATION"
TIMEOUT_SECONDS="$DEFAULT_TIMEOUT_SECONDS"
POLL_INTERVAL_MS="$DEFAULT_POLL_INTERVAL_MS"
RUN_TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
RUN_ROOT="$ROOT_DIR/.devnet/benchmarks/$RUN_TIMESTAMP"
LOG_DIR="$RUN_ROOT/logs"
BENCH_LOG_DIR="$RUN_ROOT/benchmarks"

COORDINATOR_PID=""
PROVER_PID=""

usage() {
  cat <<EOF
Usage:
  bash scripts/devnet-smoke-bench.sh [options]

This wrapper runs the current devnet smoke + benchmark sequence against the
private Chainstack devnet endpoint using the prover-backed fibonacci path.

Options:
  --no-deploy                 Skip scripts/deploy-devnet.sh
  --no-register               Skip automatic fibonacci verifier registration
  --skip-cu                   Skip scripts/benchmark-cu.ts
  --skip-cold-warm            Skip scripts/benchmark-prover-starts.ts
  --skip-load                 Skip scripts/benchmark-load.ts
  --skip-throughput           Skip scripts/benchmark-throughput.ts
  --skip-build                Skip local cargo/anchor builds
  --force-deploy              Run deploy even if required devnet programs already match local artifacts
  --keep-services             Leave local redis/coordinator/prover running after exit
  --dry-run                   Print the planned steps without executing them
  --rpc-url <url>             Solana RPC URL (default: $DEFAULT_RPC_URL)
  --ws-url <url>              Solana WebSocket URL (default: $DEFAULT_WS_URL)
  --wallet <path>             Deploy/request/coordinator wallet path
  --config <path>             Off-chain config path (default: $DEFAULT_CONFIG_PATH)
  --redis-port <port>         Local Redis port (default: $DEFAULT_REDIS_PORT)
  --fib-n <n>                 Fibonacci input for all benchmark scripts (default: $DEFAULT_FIB_N)
  --load-requests <count>     Concurrent requests for benchmark-load.ts (default: $DEFAULT_LOAD_REQUESTS)
  --throughput-tps <rate>     Target TPS for benchmark-throughput.ts (default: $DEFAULT_THROUGHPUT_TPS)
  --throughput-duration <s>   Duration for benchmark-throughput.ts (default: $DEFAULT_THROUGHPUT_DURATION)
  --timeout-seconds <s>       Callback wait timeout for CU/cold-warm benches (default: $DEFAULT_TIMEOUT_SECONDS)
  --poll-interval-ms <ms>     Poll interval for CU/cold-warm benches (default: $DEFAULT_POLL_INTERVAL_MS)
  --help                      Show this message

Outputs:
  Run logs are written under .devnet/benchmarks/<timestamp>/

Notes:
  - This wrapper intentionally defaults to fibonacci because remote devnet
    historical_avg still depends on indexed account-history data that this
    script does not provision.
  - Running with deploy enabled will send devnet transactions and may request
    an airdrop through scripts/deploy-devnet.sh.
  - The wrapper estimates required wallet balance from the enabled benchmark
    workload using SONAR_BENCHMARK_ESTIMATED_REQUEST_COST_LAMPORTS
    (default: $DEFAULT_ESTIMATED_REQUEST_COST_LAMPORTS lamports/request).
EOF
}

lamports_to_sol() {
  python3 - "$1" <<'PY'
from decimal import Decimal
import sys

lamports = Decimal(sys.argv[1])
print(f"{lamports / Decimal('1000000000'):.9f}")
PY
}

ceil_decimal_product() {
  python3 - "$1" "$2" <<'PY'
from decimal import Decimal, ROUND_CEILING
import sys

left = Decimal(sys.argv[1])
right = Decimal(sys.argv[2])
print(int((left * right).to_integral_value(rounding=ROUND_CEILING)))
PY
}

service_target_dir() {
  case "$SERVICE_PROFILE" in
    debug|dev)
      printf '%s\n' "$ROOT_DIR/target/debug"
      ;;
    release)
      printf '%s\n' "$ROOT_DIR/target/release"
      ;;
    *)
      die "unsupported service profile: $SERVICE_PROFILE (expected debug or release)"
      ;;
  esac
}

build_service_binaries() {
  case "$SERVICE_PROFILE" in
    debug|dev)
      cargo build --bin sonar-coordinator --bin sonar-prover
      ;;
    release)
      cargo build --release --bin sonar-coordinator --bin sonar-prover
      ;;
    *)
      die "unsupported service profile: $SERVICE_PROFILE (expected debug or release)"
      ;;
  esac
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

require_file() {
  [[ -f "$1" ]] || die "missing required file: $1"
}

wallet_balance_lamports() {
  local wallet_path="$1"
  local wallet_pubkey

  wallet_pubkey="$(solana-keygen pubkey "$wallet_path")"
  solana balance "$wallet_pubkey" --url "$RPC_URL" --lamports | awk 'NR==1 { print $1 }'
}

estimated_benchmark_request_count() {
  local total=0
  local throughput_requests=0

  if (( RUN_CU == 1 )); then
    total=$(( total + 1 ))
  fi

  if (( RUN_COLD_WARM == 1 )); then
    total=$(( total + 11 ))
  fi

  if (( RUN_LOAD == 1 )); then
    total=$(( total + LOAD_REQUESTS ))
  fi

  if (( RUN_THROUGHPUT == 1 )); then
    throughput_requests="$(ceil_decimal_product "$THROUGHPUT_TPS" "$THROUGHPUT_DURATION")"
    total=$(( total + throughput_requests ))
  fi

  printf '%s\n' "$total"
}

check_benchmark_wallet_balance() {
  local wallet_path="$1"
  local request_count
  local required_lamports
  local current_balance_lamports

  request_count="$(estimated_benchmark_request_count)"
  if (( request_count == 0 )); then
    log "Skipping wallet benchmark-floor check because all request-producing benchmark legs are disabled"
    return 0
  fi

  required_lamports=$(( request_count * ESTIMATED_REQUEST_COST_LAMPORTS + BENCHMARK_BALANCE_CUSHION_LAMPORTS ))

  if (( DRY_RUN == 1 )); then
    log "Dry-run: benchmark floor would require about $(lamports_to_sol "$required_lamports") SOL for ${request_count} request(s)"
    return 0
  fi

  current_balance_lamports="$(wallet_balance_lamports "$wallet_path")"
  [[ -n "$current_balance_lamports" ]] || die "failed to fetch wallet balance for benchmark preflight"

  if (( current_balance_lamports < required_lamports )); then
    if [[ "$ALLOW_LOW_BALANCE" =~ ^([Tt][Rr][Uu][Ee]|[Yy][Ee][Ss]|[Oo][Nn]|1)$ ]]; then
      log "WARNING: wallet balance $(lamports_to_sol "$current_balance_lamports") SOL is below the estimated benchmark floor $(lamports_to_sol "$required_lamports") SOL for ${request_count} request(s); continuing because SONAR_ALLOW_LOW_BENCHMARK_BALANCE=$ALLOW_LOW_BALANCE"
      return 0
    fi

    die "wallet balance $(lamports_to_sol "$current_balance_lamports") SOL is below the estimated benchmark floor $(lamports_to_sol "$required_lamports") SOL for ${request_count} request(s). Lower --load-requests/--throughput-* or override SONAR_BENCHMARK_ESTIMATED_REQUEST_COST_LAMPORTS / SONAR_BENCHMARK_BALANCE_CUSHION_LAMPORTS / SONAR_ALLOW_LOW_BENCHMARK_BALANCE=1 if you intentionally want to proceed."
  fi

  log "Wallet benchmark-floor check passed: $(lamports_to_sol "$current_balance_lamports") SOL available for an estimated ${request_count}-request workload"
}

program_binary_matches_remote() {
  local program_id="$1"
  local local_artifact="$2"
  local artifact_label="$3"
  local remote_dump_path="$RUN_ROOT/${artifact_label}.remote.so"

  if ! solana program show "$program_id" --url "$RPC_URL" >/dev/null 2>&1; then
    log "$artifact_label is not deployed on devnet"
    return 1
  fi

  if ! solana program dump "$program_id" "$remote_dump_path" --url "$RPC_URL" >/dev/null 2>&1; then
    log "Failed to dump deployed $artifact_label for comparison"
    return 1
  fi

  if cmp -s "$remote_dump_path" "$local_artifact"; then
    log "$artifact_label already matches the deployed artifact"
    return 0
  fi

  log "$artifact_label differs from the deployed artifact"
  return 1
}

expand_path() {
  local path="$1"
  if [[ "$path" == "~" ]]; then
    printf '%s\n' "$HOME"
    return
  fi

  if [[ "$path" == "~/"* ]]; then
    printf '%s/%s\n' "$HOME" "${path:2}"
    return
  fi

  printf '%s\n' "$path"
}

extract_provider_wallet() {
  awk '
    /^\[provider\]$/ { in_provider=1; next }
    /^\[/ { if (in_provider) exit; in_provider=0 }
    in_provider {
      if (match($0, /^[[:space:]]*wallet[[:space:]]*=[[:space:]]*"([^"]+)"/, m)) {
        print m[1]
        exit
      }
    }
  ' "$ANCHOR_TOML"
}

run_step() {
  local description="$1"
  shift

  log "$description"
  if (( DRY_RUN == 1 )); then
    printf '  %q' "$@"
    printf '\n'
    return 0
  fi

  "$@"
}

start_background() {
  local description="$1"
  local log_file="$2"
  shift 2

  # Callers capture stdout to read the child PID, so status output must stay off stdout.
  log "$description" >&2
  if (( DRY_RUN == 1 )); then
    printf '  %q' "$@" >&2
    printf ' > %q 2>&1 &\n' "$log_file" >&2
    printf '  # background launch skipped in dry-run mode\n' >&2
    printf '%s\n' "dry-run"
    return 0
  fi

  "$@" >"$log_file" 2>&1 &
  printf '%s\n' "$!"
}

print_log_tails() {
  local coordinator_log="$LOG_DIR/coordinator.log"
  local prover_log="$LOG_DIR/prover.log"

  if [[ -f "$coordinator_log" ]]; then
    log "Last coordinator log lines:"
    tail -n 20 "$coordinator_log" || true
  fi

  if [[ -f "$prover_log" ]]; then
    log "Last prover log lines:"
    tail -n 20 "$prover_log" || true
  fi
}

cleanup() {
  if (( KEEP_SERVICES == 1 || DRY_RUN == 1 )); then
    return 0
  fi

  if [[ -n "$COORDINATOR_PID" ]] && kill -0 "$COORDINATOR_PID" >/dev/null 2>&1; then
    log "Stopping coordinator (pid $COORDINATOR_PID)"
    kill "$COORDINATOR_PID" >/dev/null 2>&1 || true
    wait "$COORDINATOR_PID" 2>/dev/null || true
  fi

  if [[ -n "$PROVER_PID" ]] && kill -0 "$PROVER_PID" >/dev/null 2>&1; then
    log "Stopping prover (pid $PROVER_PID)"
    kill "$PROVER_PID" >/dev/null 2>&1 || true
    wait "$PROVER_PID" 2>/dev/null || true
  fi

  docker rm -f "$REDIS_CONTAINER_NAME" >/dev/null 2>&1 || true
}

trap cleanup EXIT

wait_for_process() {
  local pid="$1"
  local label="$2"
  local log_file="$3"

  if (( DRY_RUN == 1 )); then
    return 0
  fi

  for _ in $(seq 1 15); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      log "$label exited unexpectedly"
      tail -n 50 "$log_file" || true
      return 1
    fi
    sleep 1
  done

  return 0
}

wait_for_redis() {
  if (( DRY_RUN == 1 )); then
    return 0
  fi

  for _ in $(seq 1 30); do
    if docker exec "$REDIS_CONTAINER_NAME" redis-cli ping >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done

  die "timed out waiting for redis container"
}

resolve_program_id() {
  local keypair_path="$1"
  local fallback="$2"
  if [[ -f "$keypair_path" ]]; then
    solana-keygen pubkey "$keypair_path"
    return
  fi

  printf '%s\n' "$fallback"
}

build_anchor_workspace() {
  if env NO_DNA="$NO_DNA" anchor build; then
    return 0
  fi

  log "Default anchor build failed; retrying without IDL generation on Solana platform-tools v1.53"
  env NO_DNA="$NO_DNA" anchor build --no-idl -- --tools-version v1.53
}

fibonacci_computation_id_hex() {
  sha256sum "$ROOT_DIR/programs/fibonacci/elf/fibonacci-program" | awk '{print $1}'
}

verifier_registry_pda() {
  local computation_id_hex="$1"
  local program_id="$2"
  node - "$computation_id_hex" "$program_id" <<'NODE'
const { PublicKey } = require('@solana/web3.js');

const computationIdHex = process.argv[2];
const programId = new PublicKey(process.argv[3]);
const [pda] = PublicKey.findProgramAddressSync(
  [Buffer.from('verifier'), Buffer.from(computationIdHex, 'hex')],
  programId,
);

process.stdout.write(pda.toBase58());
NODE
}

ensure_fibonacci_verifier() {
  local program_id="$1"
  local wallet_path="$2"
  local computation_id_hex
  local verifier_registry

  computation_id_hex="$(fibonacci_computation_id_hex)"
  verifier_registry="$(verifier_registry_pda "$computation_id_hex" "$program_id")"

  log "Checking fibonacci verifier registry $verifier_registry"
  if (( DRY_RUN == 1 )); then
    log "Dry-run mode: skipping on-chain verifier lookup and registration"
    return 0
  fi

  if solana account "$verifier_registry" --url "$RPC_URL" >/dev/null 2>&1; then
    log "Fibonacci verifier already registered"
    return 0
  fi

  if (( RUN_REGISTER == 0 )); then
    die "fibonacci verifier registry $verifier_registry is missing; rerun without --no-register"
  fi

  run_step \
    "Registering fibonacci verifier on devnet" \
    cargo run -p sonar-cli -- register \
      --elf-path "$ROOT_DIR/programs/fibonacci/elf/fibonacci-program" \
      --keypair "$wallet_path" \
      --rpc-url "$RPC_URL"
}

while (( $# > 0 )); do
  case "$1" in
    --no-deploy)
      RUN_DEPLOY=0
      shift
      ;;
    --no-register)
      RUN_REGISTER=0
      shift
      ;;
    --skip-cu)
      RUN_CU=0
      shift
      ;;
    --skip-cold-warm)
      RUN_COLD_WARM=0
      shift
      ;;
    --skip-load)
      RUN_LOAD=0
      shift
      ;;
    --skip-throughput)
      RUN_THROUGHPUT=0
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --force-deploy)
      FORCE_DEPLOY=1
      shift
      ;;
    --keep-services)
      KEEP_SERVICES=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --rpc-url)
      RPC_URL="$2"
      shift 2
      ;;
    --ws-url)
      WS_URL="$2"
      shift 2
      ;;
    --wallet)
      ANCHOR_WALLET="$2"
      shift 2
      ;;
    --config)
      CONFIG_PATH="$2"
      shift 2
      ;;
    --redis-port)
      REDIS_PORT="$2"
      shift 2
      ;;
    --fib-n)
      FIB_N="$2"
      shift 2
      ;;
    --load-requests)
      LOAD_REQUESTS="$2"
      shift 2
      ;;
    --throughput-tps)
      THROUGHPUT_TPS="$2"
      shift 2
      ;;
    --throughput-duration)
      THROUGHPUT_DURATION="$2"
      shift 2
      ;;
    --timeout-seconds)
      TIMEOUT_SECONDS="$2"
      shift 2
      ;;
    --poll-interval-ms)
      POLL_INTERVAL_MS="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

require_command awk
require_command anchor
require_command cargo
require_command docker
require_command node
require_command npx
require_command python3
require_command sha256sum
require_command solana
require_command solana-keygen

[[ -f "$ANCHOR_TOML" ]] || die "Anchor workspace config not found at $ANCHOR_TOML"
[[ -f "$CONFIG_PATH" ]] || die "devnet smoke config not found at $CONFIG_PATH"

wallet_path="${ANCHOR_WALLET:-$(extract_provider_wallet)}"
wallet_path="${wallet_path:-~/.config/solana/id.json}"
wallet_path="$(expand_path "$wallet_path")"
[[ -f "$wallet_path" ]] || die "wallet not found at $wallet_path"

mkdir -p "$LOG_DIR" "$BENCH_LOG_DIR"

PROGRAM_ID="$DEFAULT_PROGRAM_ID"
CALLBACK_PROGRAM_ID="$DEFAULT_CALLBACK_PROGRAM_ID"

export ANCHOR_WALLET="$wallet_path"
export ANCHOR_PROVIDER_URL="$RPC_URL"
export SOLANA_RPC_URL="$RPC_URL"
export SOLANA_WS_URL="$WS_URL"
export HELIUS_RPC_URL="${HELIUS_RPC_URL:-$RPC_URL}"
export HELIUS_API_KEY="${HELIUS_API_KEY:-devnet-smoke}"
export DATABASE_URL="${DATABASE_URL:-postgresql://unused:unused@127.0.0.1:15432/unused}"
export REDIS_URL="redis://127.0.0.1:${REDIS_PORT}"
export SP1_PROVING_KEY="${SP1_PROVING_KEY:-/tmp/sp1.key}"
export GROTH16_PARAMS="${GROTH16_PARAMS:-/tmp/groth16.params}"
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-1}"
export SONAR_CONFIG_PATH="$CONFIG_PATH"
export SONAR_CONFIG="$CONFIG_PATH"
export SONAR_COORDINATOR_KEYPAIR_PATH="$wallet_path"
export SONAR_DISABLE_REQUEST_POLLING="${SONAR_DISABLE_REQUEST_POLLING:-1}"

log "Run directory: $RUN_ROOT"
log "RPC URL: $RPC_URL"
log "WS URL: $WS_URL"
log "Wallet: $wallet_path"
log "Program ID: $PROGRAM_ID"
log "Callback program ID: $CALLBACK_PROGRAM_ID"
log "Computation: fibonacci(n=$FIB_N)"
log "Local service profile: $SERVICE_PROFILE"
log "Rayon threads: $RAYON_NUM_THREADS"
log "Estimated request cost floor: ${ESTIMATED_REQUEST_COST_LAMPORTS} lamports/request"
log "Benchmark balance cushion: ${BENCHMARK_BALANCE_CUSHION_LAMPORTS} lamports"
if [[ "$SONAR_DISABLE_REQUEST_POLLING" =~ ^([Tt][Rr][Uu][Ee]|[Yy][Ee][Ss]|[Oo][Nn]|1)$ ]]; then
  log "Request polling fallback: disabled"
else
  log "Request polling fallback: enabled"
fi

SERVICE_TARGET_DIR="$(service_target_dir)"

if (( SKIP_BUILD == 0 )); then
  if (( RUN_DEPLOY == 0 )); then
    run_step "Building Anchor workspace for benchmark IDL/types" build_anchor_workspace
  fi
  run_step "Building local service binaries" build_service_binaries
fi

PROGRAM_ID="$(resolve_program_id "$ROOT_DIR/target/deploy/sonar_program-keypair.json" "$DEFAULT_PROGRAM_ID")"
CALLBACK_PROGRAM_ID="$(resolve_program_id "$ROOT_DIR/target/deploy/echo_callback-keypair.json" "$DEFAULT_CALLBACK_PROGRAM_ID")"

require_file "$ROOT_DIR/programs/fibonacci/elf/fibonacci-program"
if (( DRY_RUN == 0 )); then
  require_file "$ROOT_DIR/target/deploy/echo_callback.so"
  require_file "$ROOT_DIR/target/deploy/sonar_program.so"
  require_file "$ROOT_DIR/target/idl/sonar.json"
  require_file "$SERVICE_TARGET_DIR/sonar-coordinator"
  require_file "$SERVICE_TARGET_DIR/sonar-prover"
else
  log "Dry-run: skipping built artifact existence checks"
fi

if (( RUN_DEPLOY == 1 )); then
  if (( DRY_RUN == 1 || FORCE_DEPLOY == 1 )); then
    run_step "Deploying workspace to devnet" env NO_DNA="$NO_DNA" bash "$ROOT_DIR/scripts/deploy-devnet.sh"
  elif program_binary_matches_remote "$PROGRAM_ID" "$ROOT_DIR/target/deploy/sonar_program.so" "sonar_program" && \
    program_binary_matches_remote "$CALLBACK_PROGRAM_ID" "$ROOT_DIR/target/deploy/echo_callback.so" "echo_callback"; then
    log "Skipping deploy because the benchmark-required devnet programs already match local artifacts"
  else
    run_step "Deploying workspace to devnet" env NO_DNA="$NO_DNA" bash "$ROOT_DIR/scripts/deploy-devnet.sh"
  fi
fi

ensure_fibonacci_verifier "$PROGRAM_ID" "$wallet_path"
check_benchmark_wallet_balance "$wallet_path"

if (( DRY_RUN == 1 )); then
  log "Resetting local redis container"
  printf '  docker rm -f %q >/dev/null 2>&1 || true\n' "$REDIS_CONTAINER_NAME"
else
  log "Resetting local redis container"
  docker rm -f "$REDIS_CONTAINER_NAME" >/dev/null 2>&1 || true
fi
run_step \
  "Starting local redis for coordinator/prover" \
  docker run -d --name "$REDIS_CONTAINER_NAME" -p "${REDIS_PORT}:6379" redis:7-alpine
wait_for_redis

COORDINATOR_PID="$(start_background \
  "Starting local coordinator against devnet" \
  "$LOG_DIR/coordinator.log" \
  env NO_DNA="$NO_DNA" RUST_LOG="${RUST_LOG:-info,sonar_coordinator=debug}" "$SERVICE_TARGET_DIR/sonar-coordinator")"
wait_for_process "$COORDINATOR_PID" "coordinator" "$LOG_DIR/coordinator.log"

PROVER_PID="$(start_background \
  "Starting local prover against devnet" \
  "$LOG_DIR/prover.log" \
  env NO_DNA="$NO_DNA" "$SERVICE_TARGET_DIR/sonar-prover")"
wait_for_process "$PROVER_PID" "prover" "$LOG_DIR/prover.log"

if (( RUN_CU == 1 )); then
  run_step \
    "Running devnet callback CU smoke" \
    bash -lc "cd '$ROOT_DIR' && npx ts-node --transpile-only scripts/benchmark-cu.ts --rpc-url '$RPC_URL' --wallet '$wallet_path' --program-id '$PROGRAM_ID' --callback-program '$CALLBACK_PROGRAM_ID' --computation fibonacci --fib-n '$FIB_N' --timeout-seconds '$TIMEOUT_SECONDS' --poll-interval-ms '$POLL_INTERVAL_MS' | tee '$BENCH_LOG_DIR/benchmark-cu.log'"
fi

if (( RUN_COLD_WARM == 1 )); then
  run_step \
    "Running devnet cold-vs-warm prover benchmark" \
    bash -lc "cd '$ROOT_DIR' && npx ts-node --transpile-only scripts/benchmark-prover-starts.ts --rpc-url '$RPC_URL' --wallet '$wallet_path' --program-id '$PROGRAM_ID' --callback-program '$CALLBACK_PROGRAM_ID' --computation fibonacci --fib-n '$FIB_N' --timeout-seconds '$TIMEOUT_SECONDS' --poll-interval-ms '$POLL_INTERVAL_MS' | tee '$BENCH_LOG_DIR/benchmark-prover-starts.log'"
fi

if (( RUN_LOAD == 1 )); then
  run_step \
    "Running devnet concurrent load benchmark" \
    bash -lc "cd '$ROOT_DIR' && npx ts-node --transpile-only scripts/benchmark-load.ts --requests '$LOAD_REQUESTS' --rpc-url '$RPC_URL' --wallet '$wallet_path' --program-id '$PROGRAM_ID' --callback-program '$CALLBACK_PROGRAM_ID' --computation fibonacci --fib-n '$FIB_N' | tee '$BENCH_LOG_DIR/benchmark-load.log'"
fi

if (( RUN_THROUGHPUT == 1 )); then
  run_step \
    "Running devnet sustained throughput benchmark" \
    bash -lc "cd '$ROOT_DIR' && npx ts-node --transpile-only scripts/benchmark-throughput.ts --tps '$THROUGHPUT_TPS' --duration '$THROUGHPUT_DURATION' --rpc-url '$RPC_URL' --wallet '$wallet_path' --program-id '$PROGRAM_ID' --callback-program '$CALLBACK_PROGRAM_ID' --computation fibonacci --fib-n '$FIB_N' | tee '$BENCH_LOG_DIR/benchmark-throughput.log'"
fi

log "Devnet smoke + benchmark sequence complete"
log "Logs written to $RUN_ROOT"