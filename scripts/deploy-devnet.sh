#!/usr/bin/env bash
set -Eeuo pipefail

log() {
  printf '[deploy-devnet] %s\n' "$*"
}

die() {
  log "ERROR: $*"
  exit 1
}

on_error() {
  local exit_code=$?
  local line_number=${1:-unknown}
  log "ERROR: command failed at line ${line_number}: ${BASH_COMMAND}"
  exit "$exit_code"
}

trap 'on_error $LINENO' ERR

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANCHOR_TOML="$REPO_ROOT/Anchor.toml"
MIN_BALANCE_LAMPORTS=2000000000
POLL_ATTEMPTS=15
POLL_INTERVAL_SECONDS=2
SONAR_SOLANA_BIN="$HOME/.local/share/solana/install/active_release/bin"
DEFAULT_PRIVATE_DEVNET_RPC_URL="https://solana-devnet.core.chainstack.com/51a4443c8b33222e5327f331e007ec91"
DEPLOY_BUFFER_CUSHION_LAMPORTS=50000000
DEPLOY_MAX_SIGN_ATTEMPTS="${SONAR_DEPLOY_MAX_SIGN_ATTEMPTS:-25}"
DEPLOY_TRANSPORT="${SONAR_DEPLOY_TRANSPORT:-rpc}"
NO_DNA="${NO_DNA:-1}"

if [[ -d "$SONAR_SOLANA_BIN" ]]; then
  export PATH="$SONAR_SOLANA_BIN:$PATH"
fi

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
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

extract_devnet_program_id() {
  awk '
    /^\[programs\.devnet\]$/ { in_devnet=1; next }
    /^\[/ { if (in_devnet) exit; in_devnet=0 }
    in_devnet {
      if (match($0, /^[[:space:]]*[A-Za-z0-9_-]+[[:space:]]*=[[:space:]]*"([^"]+)"/, m)) {
        print m[1]
        exit
      }
    }
  ' "$ANCHOR_TOML"
}

get_balance_lamports() {
  local address="$1"
  local rpc_url="$2"
  local balance_output
  balance_output="$(solana balance "$address" --url "$rpc_url" --lamports)"
  printf '%s\n' "$balance_output" | awk 'NF { print $1; exit }'
}

lamports_to_sol() {
  local lamports="$1"
  python3 - "$lamports" <<'PY'
from decimal import Decimal
import sys

lamports = Decimal(sys.argv[1])
sol = lamports / Decimal("1000000000")
formatted = format(sol.normalize(), "f")
if formatted == "-0":
    formatted = "0"
print(formatted)
PY
}

wait_for_balance() {
  local address="$1"
  local minimum_lamports="$2"
  local rpc_url="$3"
  local attempt=1
  local observed

  while (( attempt <= POLL_ATTEMPTS )); do
    observed="$(get_balance_lamports "$address" "$rpc_url")"
    if [[ "$observed" =~ ^[0-9]+$ ]] && (( observed >= minimum_lamports )); then
      printf '%s\n' "$observed"
      return 0
    fi

    log "Waiting for devnet balance confirmation (${attempt}/${POLL_ATTEMPTS})"
    sleep "$POLL_INTERVAL_SECONDS"
    ((attempt++))
  done

  return 1
}

build_with_fallback() {
  if env NO_DNA="$NO_DNA" anchor build; then
    return 0
  fi

  log "Default anchor build failed; retrying without IDL generation on Solana platform-tools v1.53"
  env NO_DNA="$NO_DNA" anchor build --no-idl -- --tools-version v1.53
}

rent_lamports_for_program_artifact() {
  local artifact_path="$1"
  local artifact_bytes
  local rent_output
  local rent_sol

  artifact_bytes="$(wc -c < "$artifact_path")"
  rent_output="$(solana rent "$artifact_bytes")"
  rent_sol="$(printf '%s\n' "$rent_output" | awk '/Rent-exempt minimum:/ { print $3; exit }')"
  [[ -n "$rent_sol" ]] || die "unable to parse rent-exempt minimum for $artifact_path"

  python3 - "$rent_sol" <<'PY'
from decimal import Decimal, ROUND_UP
import sys

rent_sol = Decimal(sys.argv[1])
lamports = (rent_sol * Decimal("1000000000")).quantize(Decimal("1"), rounding=ROUND_UP)
print(int(lamports))
PY
}

largest_program_buffer_requirement() {
  local artifact
  local artifact_lamports
  local largest_lamports=0
  local largest_artifact=""

  shopt -s nullglob
  for artifact in "$REPO_ROOT"/target/deploy/*.so; do
    artifact_lamports="$(rent_lamports_for_program_artifact "$artifact")"
    if (( artifact_lamports > largest_lamports )); then
      largest_lamports="$artifact_lamports"
      largest_artifact="$artifact"
    fi
  done
  shopt -u nullglob

  [[ -n "$largest_artifact" ]] || die "no deployable program artifacts found under $REPO_ROOT/target/deploy"
  printf '%s %s\n' "$largest_lamports" "$largest_artifact"
}

deploy_transport_args() {
  case "$DEPLOY_TRANSPORT" in
    rpc)
      printf '%s\n' --use-rpc
      ;;
    quic)
      printf '%s\n' --use-quic
      ;;
    tpu)
      printf '%s\n' --use-tpu-client
      ;;
    udp)
      printf '%s\n' --use-udp
      ;;
    *)
      die "unsupported SONAR_DEPLOY_TRANSPORT: $DEPLOY_TRANSPORT"
      ;;
  esac
}

require_command awk
require_command anchor
require_command python3
require_command solana
require_command solana-keygen

[[ -f "$ANCHOR_TOML" ]] || die "Anchor workspace config not found at $ANCHOR_TOML"

wallet_path="${ANCHOR_WALLET:-$(extract_provider_wallet)}"
wallet_path="${wallet_path:-~/.config/solana/id.json}"
wallet_path="$(expand_path "$wallet_path")"
[[ -f "$wallet_path" ]] || die "deployment wallet not found at $wallet_path"

export ANCHOR_WALLET="$wallet_path"
wallet_pubkey="$(solana address -k "$ANCHOR_WALLET")"
deploy_rpc_url="${SOLANA_RPC_URL:-${ANCHOR_PROVIDER_URL:-$DEFAULT_PRIVATE_DEVNET_RPC_URL}}"
current_devnet_program_id="$(extract_devnet_program_id || true)"
program_keypair="$REPO_ROOT/target/deploy/sonar_program-keypair.json"
program_pubkey=""

if [[ -f "$program_keypair" ]]; then
  program_pubkey="$(solana-keygen pubkey "$program_keypair")"
fi

log "Using Anchor wallet $ANCHOR_WALLET"
log "Deploy payer pubkey: $wallet_pubkey"

solana config set --url "$deploy_rpc_url" >/dev/null
log "Solana CLI RPC URL set to $deploy_rpc_url"

current_balance_lamports="$(get_balance_lamports "$wallet_pubkey" "$deploy_rpc_url")"
[[ "$current_balance_lamports" =~ ^[0-9]+$ ]] || die "unable to parse wallet balance: $current_balance_lamports"

if (( current_balance_lamports < MIN_BALANCE_LAMPORTS )); then
  required_lamports=$(( MIN_BALANCE_LAMPORTS - current_balance_lamports ))
  required_sol="$(lamports_to_sol "$required_lamports")"
  log "Wallet balance below 2 SOL; requesting ${required_sol} SOL airdrop"
  if ! solana airdrop "$required_sol" "$wallet_pubkey" --url "$deploy_rpc_url" >/dev/null; then
    die "wallet balance is only $(lamports_to_sol "$current_balance_lamports") SOL and the devnet airdrop request failed; fund $wallet_pubkey manually or retry once the faucet/rate limit clears"
  fi
  current_balance_lamports="$(wait_for_balance "$wallet_pubkey" "$MIN_BALANCE_LAMPORTS" "$deploy_rpc_url")" || die "airdrop did not raise wallet balance to at least 2 SOL"
fi

log "Wallet balance: $(lamports_to_sol "$current_balance_lamports") SOL"

read -r largest_buffer_lamports largest_buffer_artifact < <(largest_program_buffer_requirement)
recommended_balance_lamports=$(( largest_buffer_lamports + DEPLOY_BUFFER_CUSHION_LAMPORTS ))
if (( current_balance_lamports < recommended_balance_lamports )); then
  die "wallet balance $(lamports_to_sol "$current_balance_lamports") SOL is below the recommended deploy floor $(lamports_to_sol "$recommended_balance_lamports") SOL; the largest workspace program buffer ($(basename "$largest_buffer_artifact")) requires about $(lamports_to_sol "$largest_buffer_lamports") SOL before transaction fees"
fi

if [[ -n "$current_devnet_program_id" && -n "$program_pubkey" && "$current_devnet_program_id" != "$program_pubkey" ]]; then
  log "WARNING: Anchor.toml devnet program ID ($current_devnet_program_id) does not match target/deploy/sonar_program-keypair.json ($program_pubkey)"
  log "WARNING: anchor keys sync and anchor deploy will follow the target/deploy keypair unless you replace that keypair"
fi

log "Synchronizing Anchor program keys"
env NO_DNA="$NO_DNA" anchor keys sync

log "Building Anchor workspace"
build_with_fallback

log "Deploying Anchor workspace to devnet"
log "Using deploy transport '$DEPLOY_TRANSPORT' with max sign attempts $DEPLOY_MAX_SIGN_ATTEMPTS"
env NO_DNA="$NO_DNA" anchor deploy --provider.cluster "$deploy_rpc_url" --provider.wallet "$ANCHOR_WALLET" -- --max-sign-attempts "$DEPLOY_MAX_SIGN_ATTEMPTS" "$(deploy_transport_args)"

if [[ -n "$program_pubkey" ]]; then
  log "Verifying deployed program account $program_pubkey"
  solana program show "$program_pubkey" --url "$deploy_rpc_url" >/dev/null
  log "Deployment complete for sonar_program: $program_pubkey"
else
  log "Deployment complete"
fi
