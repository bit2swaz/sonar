#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cd "$REPO_ROOT"

# Anchor CLI v0.32 streams logs using the crate lib name, but the Sonar IDL is
# emitted as `target/idl/sonar.json` because the #[program] module is named
# `sonar`. Keep a synced alias at the lib-name path so `anchor test` can start
# log streaming without tripping over ENOENT after the suite passes.
SOURCE_IDL="target/idl/sonar.json"
TARGET_IDL="target/idl/sonar_program.json"

if [[ ! -f "$SOURCE_IDL" ]]; then
  echo "missing $SOURCE_IDL; run anchor build before syncing IDL aliases" >&2
  exit 1
fi

if [[ ! -f "$TARGET_IDL" ]] || ! cmp -s "$SOURCE_IDL" "$TARGET_IDL"; then
  cp "$SOURCE_IDL" "$TARGET_IDL"
fi