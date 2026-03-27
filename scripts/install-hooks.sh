#!/bin/sh
# install sonar git hooks
# run once after cloning: sh scripts/install-hooks.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cp "$SCRIPT_DIR/pre-commit" "$REPO_ROOT/.git/hooks/pre-commit"
chmod +x "$REPO_ROOT/.git/hooks/pre-commit"

echo "Git hooks installed successfully."
echo "  pre-commit → .git/hooks/pre-commit"
