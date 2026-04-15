#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"${repo_root}/scripts/cleanup_stale_rusty_v8.sh"
"${repo_root}/scripts/ensure_rusty_v8_archive.sh"

cd "${repo_root}"
cargo build -p codex-cli "$@"
