#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

artifact_dir="$tmp_dir/artifacts"
binary_name="opengpu-aarch64-apple-darwin"
binary_path="$artifact_dir/$binary_name"
agent_name="opengpu-node-agent-aarch64-apple-darwin"
agent_path="$artifact_dir/$agent_name"
private_key="$tmp_dir/private.pem"
public_key="$tmp_dir/public.pem"

if command -v python3 >/dev/null 2>&1; then
  python_bin="python3"
elif command -v python >/dev/null 2>&1; then
  python_bin="python"
else
  echo "python3 or python is required" >&2
  exit 1
fi

mkdir -p "$artifact_dir"
printf '#!/usr/bin/env bash\necho opengpu 0.1.0\n' >"$binary_path"
printf '#!/usr/bin/env bash\necho opengpu-node-agent 0.1.0\n' >"$agent_path"
chmod +x "$binary_path"
chmod +x "$agent_path"

if command -v shasum >/dev/null 2>&1; then
  checksum="$(shasum -a 256 "$binary_path" | awk '{print $1}')"
  agent_checksum="$(shasum -a 256 "$agent_path" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  checksum="$(sha256sum "$binary_path" | awk '{print $1}')"
  agent_checksum="$(sha256sum "$agent_path" | awk '{print $1}')"
else
  echo "missing checksum tool" >&2
  exit 1
fi

printf '%s  %s\n' "$checksum" "$binary_name" >"$binary_path.sha256"
printf '%s  %s\n' "$agent_checksum" "$agent_name" >"$agent_path.sha256"

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$private_key" >/dev/null 2>&1
openssl pkey -in "$private_key" -pubout -out "$public_key" >/dev/null 2>&1

private_key_b64="$("$python_bin" - "$private_key" <<'PY'
import base64
import sys
from pathlib import Path

print(base64.b64encode(Path(sys.argv[1]).read_bytes()).decode("ascii"))
PY
)"

public_key_b64="$("$python_bin" - "$public_key" <<'PY'
import base64
import sys
from pathlib import Path

print(base64.b64encode(Path(sys.argv[1]).read_bytes()).decode("ascii"))
PY
)"

prepare_output="$(
  OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64="$private_key_b64" \
  OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64="$public_key_b64" \
  "$repo_root/scripts/release-signing.sh" prepare "$artifact_dir" "$binary_name" "cli-v0.1.0" "0.1.0"
)"

if grep -F "local generated preview keys" <<<"$prepare_output" >/dev/null; then
  echo "CI signing secrets unexpectedly fell back to local generated preview keys" >&2
  exit 1
fi

OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64="$public_key_b64" \
  "$repo_root/scripts/release-signing.sh" verify "$artifact_dir" "$binary_name" >/dev/null

"$python_bin" - "$artifact_dir/release-manifest.json" "$agent_name" "$agent_checksum" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
agent_name = sys.argv[2]
agent_checksum = sys.argv[3]
assets = manifest.get("assets", [])
assert assets, "manifest did not include node-agent assets"
asset = assets[0]
assert asset["name"] == agent_name
assert asset["install_as"] == "opengpu-node-agent"
assert asset["kind"] == "node-agent-binary"
assert asset["checksum_sha256"] == agent_checksum
PY

if OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64="not-base64" \
  OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64="$public_key_b64" \
  "$repo_root/scripts/release-signing.sh" prepare "$artifact_dir" "$binary_name" "cli-v0.1.0" "0.1.0" >"$tmp_dir/malformed.out" 2>"$tmp_dir/malformed.err"; then
  echo "malformed private key base64 should fail" >&2
  exit 1
fi

if ! grep -F "FAILED: invalid base64 release signing private-key" "$tmp_dir/malformed.err" >/dev/null; then
  echo "malformed private key base64 did not report the expected error" >&2
  cat "$tmp_dir/malformed.err" >&2
  exit 1
fi

echo "Release signing CI key path verified."
