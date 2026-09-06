#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/no-auth-e2e-smoke.sh"

rg -q "MUNDUSX_AUTH_DISABLED=true" "$script"
rg -q "python_bin" "$script"
rg -q "operator_auth_enforced=false" "$script"
rg -q '\$agent" run --once' "$script"
rg -q "jobs submit" "$script"
rg -q "jobs wait" "$script"
rg -q "No-auth end-to-end smoke complete" "$script"

echo "no-auth e2e smoke contract is complete"
