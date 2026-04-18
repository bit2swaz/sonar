#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKIP_BUILD="${SONAR_SKIP_ANCHOR_BUILD:-0}"
NO_DNA="${NO_DNA:-1}"

cd "$REPO_ROOT"

if [[ "$SKIP_BUILD" != "1" ]]; then
  NO_DNA="$NO_DNA" anchor build
fi

bash "$REPO_ROOT/scripts/sync-anchor-idl-aliases.sh"
NO_DNA="$NO_DNA" anchor test --skip-build "$@"