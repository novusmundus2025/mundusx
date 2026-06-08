#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dashboard_url="http://127.0.0.1:3312"
docs_dir="$repo_root/dist/public-docs-site"

rm -rf "$docs_dir"

output="$(
  DASHBOARD_URL="$dashboard_url" \
  SMOKE_SKIP_CONTROL_PLANE=1 \
  SMOKE_SKIP_RELEASE=1 \
  SMOKE_SKIP_INSTALL=1 \
  bash "$repo_root/scripts/localhost-smoke.sh" 2>&1
)"

printf '%s\n' "$output"

printf '%s' "$output" | rg -q "Smoke test complete."
if printf '%s' "$output" | rg -q "falling back to static file verification"; then
  rg -q "Install MundusX on your Mac" "$docs_dir/public/install/index.html"
fi
