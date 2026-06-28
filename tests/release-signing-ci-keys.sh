#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

artifact_dir="$tmp_dir/artifacts"
binary_name="opengpu-aarch64-apple-darwin"
binary_path="$artifact_dir/$binary_name"
private_key="$tmp_dir/private.pem"
public_key="$tmp_dir/public.pem"

mkdir -p "$artifact_dir"
printf '#!/usr/bin/env bash\necho opengpu 0.1.0\n' >"$binary_path"
chmod +x "$binary_path"

if command -v shasum >/dev/null 2>&1; then
  checksum="$(shasum -a 256 "$binary_path" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  checksum="$(sha256sum "$binary_path" | awk '{print $1}')"
else
  echo "missing checksum tool" >&2
  exit 1
fi

printf '%s  %s\n' "$checksum" "$binary_name" >"$binary_path.sha256"

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$private_key" >/dev/null 2>&1
openssl pkey -in "$private_key" -pubout -out "$public_key" >/dev/null 2>&1

private_key_b64="$(python3 - "$private_key" <<'PY'
import base64
import sys
from pathlib import Path

print(base64.b64encode(Path(sys.argv[1]).read_bytes()).decode("ascii"))
PY
)"

public_key_b64="$(python3 - "$public_key" <<'PY'
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
