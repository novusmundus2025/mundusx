#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

require_line() {
  local path="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$repo_root/$path"; then
    echo "missing pattern '$pattern' in $path" >&2
    exit 1
  fi
}

reject_line() {
  local path="$1"
  local pattern="$2"
  if grep -Fq "$pattern" "$repo_root/$path"; then
    echo "unexpected pattern '$pattern' in $path" >&2
    exit 1
  fi
}

require_line "README.md" "http://127.0.0.1:3002/install.json"
require_line "scripts/localhost-smoke.sh" 'dashboard_url="${DASHBOARD_URL:-http://127.0.0.1:3002}"'
require_line "docs/install-page.md" 'The local dashboard typically runs on `3002`'
reject_line "docs/install-page.md" 'The local dashboard typically runs on `3001`'

echo "install docs match the default localhost dashboard port"
