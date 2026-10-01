#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
# Load only the progress helpers; never run installation or modify the host.
sed -n '/^progress_pid=/,/^trap stop_progress EXIT/p' "$root/install.sh" > "$tmp/helpers.sh"
source "$tmp/helpers.sh"
trap 'stop_progress; rm -rf "$tmp"' EXIT
sleep() { if [ "$1" = 10 ]; then command sleep 0.1; else command sleep "$@"; fi; }
run_step "Slow fixture" bash -c 'sleep 0.3; printf payload' > "$tmp/out" 2> "$tmp/log"
[ "$(cat "$tmp/out")" = payload ]
grep -Fq 'still running' "$tmp/log"
grep -Fq 'complete.' "$tmp/log"
[ -z "$progress_pid" ]
status=0
run_step "Failure fixture" bash -c 'exit 7' 2> "$tmp/failure" || status=$?
[ "$status" -eq 7 ]
grep -Fq 'failed (exit 7)' "$tmp/failure"
[ -z "$progress_pid" ]
echo 'Progress heartbeat, output preservation, and failure propagation verified.'
