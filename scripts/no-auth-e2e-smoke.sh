#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
control_plane_repo="${CONTROL_PLANE_REPO:-$(cd "$repo_root/.." && pwd)/control-plane}"
control_plane_url="${CONTROL_PLANE_URL:-http://127.0.0.1:8787}"
control_plane_host="${CONTROL_PLANE_HOST:-127.0.0.1}"
control_plane_port="${CONTROL_PLANE_PORT:-8787}"
smoke_home="${OPENGPU_SMOKE_HOME:-${TMPDIR:-/tmp}/opengpu-no-auth-e2e-smoke}"
smoke_bin="$smoke_home/bin"
smoke_model="$smoke_home/models/tiny-smoke.gguf"
control_plane_pid=""
python_bin=()

if command -v python3 >/dev/null 2>&1; then
  python_bin=(python3)
elif command -v python >/dev/null 2>&1; then
  python_bin=(python)
elif command -v py >/dev/null 2>&1; then
  python_bin=(py -3)
else
  echo "FAILED: python3, python, or py is required for JSON smoke assertions" >&2
  exit 1
fi

cleanup() {
  if [ -n "$control_plane_pid" ] && kill -0 "$control_plane_pid" 2>/dev/null; then
    kill "$control_plane_pid" >/dev/null 2>&1 || true
    wait "$control_plane_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT

die() {
  echo "FAILED: $*" >&2
  exit 1
}

require_file() {
  [ -f "$1" ] || die "$2: $1"
}

json_get() {
  "${python_bin[@]}" - "$1" "$2" <<'PY'
import json
import sys

path = sys.argv[1].split(".")
value = json.load(open(sys.argv[2], encoding="utf-8"))
for part in path:
    value = value[part]
print(value)
PY
}

wait_for_url() {
  local url="$1"
  local label="$2"

  for _ in $(seq 1 60); do
    if curl -fsSL --max-time 2 "$url" >/dev/null 2>&1; then
      echo "OK: $label"
      return
    fi
    sleep 0.5
  done

  die "$label did not become ready at $url"
}

write_llama_stub() {
  mkdir -p "$smoke_bin"
  cat >"$smoke_bin/llama-cli" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

if [ "${1:-}" = "--list-devices" ]; then
  echo "Device 0: BLAS CPU smoke device"
  exit 0
fi

prompt=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -p|--prompt)
      shift
      prompt="${1:-}"
      ;;
  esac
  shift || true
done

printf '%s\n' "$prompt"
printf '%s\n' "no-auth smoke response"
SH
  chmod +x "$smoke_bin/llama-cli"
}

build_binaries() {
  echo "Building CLI and node-agent binaries..."
  cargo build --manifest-path "$repo_root/apps/cli/Cargo.toml"
  cargo build --manifest-path "$repo_root/agents/node/Cargo.toml"
}

binary_path() {
  local name="$1"

  if [ -x "$repo_root/target/debug/$name" ]; then
    printf '%s\n' "$repo_root/target/debug/$name"
    return
  fi

  if [ -x "$repo_root/target/debug/$name.exe" ]; then
    printf '%s\n' "$repo_root/target/debug/$name.exe"
    return
  fi

  die "expected binary not found after build: $name"
}

start_control_plane() {
  require_file "$control_plane_repo/Cargo.toml" "control-plane repo is required"

  echo "Starting control plane with operator auth explicitly disabled..."
  (
    cd "$control_plane_repo"
    MUNDUSX_AUTH_DISABLED=true \
      MUNDUSX_CONTROL_PLANE_HOME="$smoke_home/control-plane-home" \
      HOST="$control_plane_host" \
      PORT="$control_plane_port" \
      cargo run -p opengpu-control-plane
  ) >"$smoke_home/control-plane.log" 2>&1 &
  control_plane_pid="$!"

  wait_for_url "$control_plane_url/health" "control plane health"
}

assert_health_no_auth() {
  local health_file="$smoke_home/health.json"

  curl -fsSL "$control_plane_url/health" >"$health_file"
  [ "$(json_get operator_auth_enforced "$health_file")" = "False" ] ||
    [ "$(json_get operator_auth_enforced "$health_file")" = "false" ] ||
    die "/health did not report operator_auth_enforced=false"
  echo "OK: operator auth disabled on /health"
}

seed_contributor_home() {
  local cli

  cli="$(binary_path opengpu)"

  rm -rf "$smoke_home"
  mkdir -p "$(dirname "$smoke_model")"
  printf '%s\n' "tiny local smoke model" >"$smoke_model"
  write_llama_stub

  echo "Seeding isolated OPENGPU_HOME..."
  OPENGPU_HOME="$smoke_home" "${python_bin[@]}" - "$control_plane_url" "$smoke_home/models" <<'PY'
import json
import os
import pathlib
import sys
import uuid

home = pathlib.Path(os.environ["OPENGPU_HOME"])
home.mkdir(parents=True, exist_ok=True)
config = {
    "version": 1,
    "device_id": f"node-{uuid.uuid4().hex}",
    "public_key_fingerprint": None,
    "profile_name": None,
    "auth_token": None,
    "connected": False,
    "paused": False,
    "backend_preference": "m",
    "contribution_percent": 20,
    "control_plane_url": sys.argv[1],
    "active_model": None,
    "models": [],
    "model_dir": sys.argv[2],
    "onboarding_completed": False,
}
(home / "config.json").write_text(json.dumps(config, indent=2), encoding="utf-8")
PY
  OPENGPU_HOME="$smoke_home" \
    "$cli" model import "$smoke_model" \
    --name tiny-smoke \
    --backend m \
    --activate >/tmp/opengpu-no-auth-model.log
  OPENGPU_HOME="$smoke_home" "$cli" onboarding --complete >/dev/null
  OPENGPU_HOME="$smoke_home" "$cli" start >/tmp/opengpu-no-auth-start.log
  echo "OK: contributor home seeded"
}

run_lifecycle() {
  local cli
  local agent
  local job_file="$smoke_home/job-submit.json"
  local claim_file="$smoke_home/claim.json"
  local wait_file="$smoke_home/job-wait.json"
  local job_id

  cli="$(binary_path opengpu)"
  agent="$(binary_path opengpu-node-agent)"

  echo "Registering and heartbeating signed node..."
  OPENGPU_HOME="$smoke_home" PATH="$smoke_bin:$PATH" "$agent" run --once >/tmp/opengpu-no-auth-agent-once.log 2>&1
  if rg -q "controlPlaneRegister: HTTP" /tmp/opengpu-no-auth-agent-once.log; then
    cat /tmp/opengpu-no-auth-agent-once.log
    die "signed node registration failed"
  fi
  curl -fsSL "$control_plane_url/v1/nodes" | rg -q "tiny-smoke" ||
    die "registered node with active model was not visible"
  echo "OK: register and heartbeat visible"

  echo "Submitting job without operator auth..."
  OPENGPU_HOME="$smoke_home" "$cli" jobs submit \
    --model tiny-smoke \
    --prompt "no-auth smoke prompt" \
    --json >"$job_file"
  job_id="$(json_get job_id "$job_file")"
  [ -n "$job_id" ] || die "job submit did not return job_id"
  echo "OK: job submitted $job_id"

  echo "Claiming and completing job through node-agent..."
  OPENGPU_HOME="$smoke_home" PATH="$smoke_bin:$PATH" "$agent" run --once >"$claim_file" 2>&1
  rg -q "jobPoll: claimed $job_id" "$claim_file" || die "node-agent did not claim or report a job"

  OPENGPU_HOME="$smoke_home" "$cli" jobs wait "$job_id" --timeout 20 --interval 1 --json >"$wait_file"
  [ "$(json_get status "$wait_file")" = "completed" ] || die "job did not complete"
  rg -q "no-auth smoke response" "$wait_file" || die "completed job output was missing"
  echo "OK: claim, complete, and poll result"
}

build_binaries
seed_contributor_home
start_control_plane
assert_health_no_auth
run_lifecycle

echo "No-auth end-to-end smoke complete."
