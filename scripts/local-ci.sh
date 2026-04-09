#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SECRETS_FILE="${ROOT_DIR}/.secrets"

if ! command -v docker >/dev/null 2>&1; then
  echo "Error: docker is not installed or not in PATH." >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "Error: Docker daemon is not running or not reachable." >&2
  echo "Start Docker and try again." >&2
  exit 1
fi

if ! command -v act >/dev/null 2>&1; then
  echo "Error: act is not installed or not in PATH." >&2
  echo "Install act first, then re-run this script." >&2
  exit 1
fi

if [[ ! -f "${SECRETS_FILE}" ]]; then
  echo "Error: ${SECRETS_FILE} does not exist." >&2
  echo "Create it by copying .secrets.example to .secrets and updating any values you need." >&2
  exit 1
fi

cd "${ROOT_DIR}"
exec act --secret-file .secrets --action-offline-mode --pull=false "$@"
