#!/usr/bin/env bash

# Exercises the "a local LLM cluster is already running" path end to end against
# a stub OpenAI-compatible endpoint: detection, contribution, and forgetting.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v python3 >/dev/null 2>&1; then
  echo "SKIP: python3 is required to serve the stub cluster endpoint" >&2
  exit 0
fi

port="${CLUSTER_SMOKE_PORT:-18234}"
base_url="http://127.0.0.1:${port}"
work_dir="$(mktemp -d)"
stub_pid=""

cleanup() {
  if [[ -n "$stub_pid" ]] && kill -0 "$stub_pid" 2>/dev/null; then
    kill "$stub_pid" 2>/dev/null || true
    wait "$stub_pid" 2>/dev/null || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"

  if [[ "$haystack" != *"$needle"* ]]; then
    echo "expected $label to contain: $needle" >&2
    echo "actual: $haystack" >&2
    exit 1
  fi
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  local label="$3"

  if [[ "$haystack" == *"$needle"* ]]; then
    echo "did not expect $label to contain: $needle" >&2
    echo "actual: $haystack" >&2
    exit 1
  fi
}

cargo build -p opengpu >/dev/null
cli="$repo_root/target/debug/opengpu"

python3 - "$port" <<'PY' &
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

# llama.cpp-shaped listing: the 70B must win over the 7B on parameter count,
# even though the 7B is listed first.
MODELS = {
    "data": [
        {"id": "qwen2.5-7b", "meta": {"n_params": 7_000_000_000, "size": 4_000_000_000}},
        {"id": "llama-3.1-70b", "meta": {"n_params": 70_000_000_000, "size": 40_000_000_000}},
    ]
}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/v1/models":
            self.send_error(404)
            return
        body = json.dumps(MODELS).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY
stub_pid=$!

for _ in $(seq 1 50); do
  if curl -sf "${base_url}/v1/models" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

if ! curl -sf "${base_url}/v1/models" >/dev/null 2>&1; then
  echo "stub cluster endpoint never came up on ${base_url}" >&2
  exit 1
fi

export OPENGPU_HOME="$work_dir/home"
export OPENGPU_CLUSTER_PROBE_URLS="$base_url"
mkdir -p "$OPENGPU_HOME"

config_path="$OPENGPU_HOME/config.json"

# save_config always writes the CWD-relative `.opengpu/config.json` fallback in
# addition to OPENGPU_HOME, so run the CLI from a scratch directory instead of
# the repo root.
mkdir -p "$work_dir/cwd"
cd "$work_dir/cwd"

scan="$("$cli" cluster scan --json)"
assert_contains "$scan" "$base_url" "cluster scan"
assert_contains "$scan" '"servable": true' "cluster scan"
assert_contains "$scan" "qwen2.5-7b" "cluster scan"
# Size ranking must pick the 70B over the 7B listed before it.
assert_contains "$scan" '"largest_model": "llama-3.1-70b"' "cluster scan"

install_output="$("$cli" install --public --cap-percent 30 --contribute-cluster)"
assert_contains "$install_output" "CLUSTER CONTRIBUTED" "install output"
assert_contains "$install_output" "$base_url" "install output"

config="$(cat "$config_path")"
assert_contains "$config" '"contributed_cluster"' "config"
assert_contains "$config" "$base_url" "config"
assert_contains "$config" "qwen2.5-7b" "config"
# The node advertises the biggest model the cluster serves, not the first one.
assert_contains "$config" '"model": "llama-3.1-70b"' "config"
# Contributing a running cluster must not download or activate a MundusX model.
assert_contains "$config" '"active_model": null' "config"

status="$("$cli" status --json)"
assert_contains "$status" '"contributed_cluster"' "status"
assert_contains "$status" '"active_model": "llama-3.1-70b"' "status"

doctor="$("$cli" doctor --json)"
assert_contains "$doctor" '"active_model": "llama-3.1-70b"' "doctor"

# A second setup run must not re-ask or clobber the recorded decision.
"$cli" install --public --cap-percent 30 >/dev/null
config="$(cat "$config_path")"
assert_contains "$config" "$base_url" "config after re-install"

forget_output="$("$cli" cluster forget)"
assert_contains "$forget_output" "clusterForgotten" "forget output"
config="$(cat "$config_path")"
assert_contains "$config" '"contributed_cluster": null' "config after forget"

# An explicit --model overrides the biggest-model default.
use_output="$("$cli" cluster use "$base_url" --model qwen2.5-7b)"
assert_contains "$use_output" "CLUSTER CONTRIBUTED" "cluster use output"
config="$(cat "$config_path")"
assert_contains "$config" '"model": "qwen2.5-7b"' "config after cluster use"

if "$cli" cluster use "$base_url" --model not-served >/dev/null 2>"$work_dir/err.txt"; then
  echo 'expected `cluster use` to reject a model the cluster does not serve' >&2
  exit 1
fi
assert_contains "$(cat "$work_dir/err.txt")" "is not served by" "cluster use error"

if "$cli" cluster use "127.0.0.1:1234" >/dev/null 2>"$work_dir/err.txt"; then
  echo 'expected `cluster use` to reject a URL without a scheme' >&2
  exit 1
fi
assert_contains "$(cat "$work_dir/err.txt")" "must be a full http:// or https:// URL" "cluster use error"

# Detection stays off when the contributor opts out through the environment.
scan_disabled="$(OPENGPU_SKIP_CLUSTER_DETECT=1 "$cli" cluster scan --json)"
assert_contains "$scan_disabled" '"detection_disabled": true' "disabled scan"
assert_not_contains "$scan_disabled" '"servable"' "disabled scan"

echo "PASS: opengpu detects, contributes, and forgets a running local cluster"
