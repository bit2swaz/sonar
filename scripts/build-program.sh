#!/usr/bin/env bash
# Build the Anchor program using platform-tools v1.53 (Rust/Cargo 1.89).
#
# Background: cargo-build-sbf 3.0.13 (Solana CLI 3.0.13) defaults to
# platform-tools v1.51 which ships with Cargo 1.84. Modern crate dependencies
# (blake3, toml_edit, …) now require Cargo ≥1.85 (edition2024 support).
# platform-tools v1.53 ships Rust/Cargo 1.89 and resolves this.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Ensure Solana toolchain is on PATH (Anchor CLI needs it).
if [[ -d "$HOME/.local/share/solana/install/active_release/bin" ]]; then
  export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"
fi

# Build the program .so.
cd "$REPO_ROOT"
cargo-build-sbf --tools-version v1.53 --manifest-path program/Cargo.toml

echo "Build successful: target/deploy/sonar_program.so"
