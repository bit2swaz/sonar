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

if [[ -d "$SONAR_SOLANA_BIN" ]]; then
  export PATH="$SONAR_SOLANA_BIN:$PATH"
fi

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

expand_path() {
  local path="$1"
  case "$path" in
    ~) printf '%s\n' "$HOME" ;;
    ~/*) printf '%s/%s\n' "$HOME" "${path#~/}" ;;
    *) printf '%s\n' "$path" ;;
  esac
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
  local balance_output
  balance_output="$(solana balance "$address" --url devnet --lamports)"
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
  local attempt=1
  local observed

  while (( attempt <= POLL_ATTEMPTS )); do
    observed="$(get_balance_lamports "$address")"
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
  if anchor build; then
    return 0
  fi

  log "Default anchor build failed; retrying with Solana platform-tools v1.53"
  anchor build -- --tools-version v1.53
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
current_devnet_program_id="$(extract_devnet_program_id || true)"
program_keypair="$REPO_ROOT/target/deploy/sonar_program-keypair.json"
program_pubkey=""

if [[ -f "$program_keypair" ]]; then
  program_pubkey="$(solana-keygen pubkey "$program_keypair")"
fi

log "Using Anchor wallet $ANCHOR_WALLET"
log "Deploy payer pubkey: $wallet_pubkey"

solana config set --url devnet >/dev/null
log "Solana CLI RPC URL set to devnet"

current_balance_lamports="$(get_balance_lamports "$wallet_pubkey")"
[[ "$current_balance_lamports" =~ ^[0-9]+$ ]] || die "unable to parse wallet balance: $current_balance_lamports"

if (( current_balance_lamports < MIN_BALANCE_LAMPORTS )); then
  required_lamports=$(( MIN_BALANCE_LAMPORTS - current_balance_lamports ))
  required_sol="$(lamports_to_sol "$required_lamports")"
  log "Wallet balance below 2 SOL; requesting ${required_sol} SOL airdrop"
  solana airdrop "$required_sol" "$wallet_pubkey" --url devnet >/dev/null
  current_balance_lamports="$(wait_for_balance "$wallet_pubkey" "$MIN_BALANCE_LAMPORTS")" || die "airdrop did not raise wallet balance to at least 2 SOL"
fi

log "Wallet balance: $(lamports_to_sol "$current_balance_lamports") SOL"

if [[ -n "$current_devnet_program_id" && -n "$program_pubkey" && "$current_devnet_program_id" != "$program_pubkey" ]]; then
  log "WARNING: Anchor.toml devnet program ID ($current_devnet_program_id) does not match target/deploy/sonar_program-keypair.json ($program_pubkey)"
  log "WARNING: anchor keys sync and anchor deploy will follow the target/deploy keypair unless you replace that keypair"
fi

log "Synchronizing Anchor program keys"
anchor keys sync

log "Building Anchor workspace"
build_with_fallback

log "Deploying Anchor workspace to devnet"
anchor deploy --provider.cluster devnet

if [[ -n "$program_pubkey" ]]; then
  log "Verifying deployed program account $program_pubkey"
  solana program show "$program_pubkey" --url devnet >/dev/null
  log "Deployment complete for sonar_program: $program_pubkey"
else
  log "Deployment complete"
fi
