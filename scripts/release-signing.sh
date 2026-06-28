#!/usr/bin/env bash
set -euo pipefail

command="${1:-}"
artifact_dir="${2:-}"
binary_name="${3:-}"
tag_name="${4:-}"
version="${5:-}"

usage() {
  cat <<'EOF'
Usage: scripts/release-signing.sh <prepare|verify> <artifact-dir> <binary-name> [tag] [version]

Commands:
  prepare  Create a signed release manifest for the artifact directory.
  verify   Verify the release manifest signature for the artifact directory.
EOF
}

die() {
  echo "FAILED: $*" >&2
  exit 1
}

if [ -z "$command" ] || [ -z "$artifact_dir" ] || [ -z "$binary_name" ]; then
  usage >&2
  exit 1
fi

manifest_path="$artifact_dir/release-manifest.json"
signature_path="$artifact_dir/release-manifest.json.sig"
private_key_path="${artifact_dir}/.signing/private.pem"
public_key_path="${artifact_dir}/.signing/public.pem"
generated_keys=0

normalize_text() {
  local value="$1"
  if [ -n "$value" ]; then
    printf '%s' "$value"
  else
    printf 'local-preview'
  fi
}

checksum_for_binary() {
  local checksum_path="$artifact_dir/$binary_name.sha256"
  [ -f "$checksum_path" ] || die "missing checksum file: $checksum_path"

  awk '{print $1}' "$checksum_path" | head -n 1
}

create_manifest() {
  local checksum tag_text version_text generated_at
  checksum="$(checksum_for_binary)"
  tag_text="$(normalize_text "$tag_name")"
  version_text="$(normalize_text "$version")"
  generated_at="$(python3 - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"))
PY
)"

  python3 - "$manifest_path" "$binary_name" "$checksum" "$tag_text" "$version_text" "$generated_at" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
binary_name = sys.argv[2]
checksum = sys.argv[3]
tag = sys.argv[4]
version = sys.argv[5]
generated_at = sys.argv[6]

payload = {
    "artifact_kind": "release-binary",
    "binary_name": binary_name,
    "checksum_sha256": checksum,
    "generated_at": generated_at,
    "tag": tag,
    "version": version,
}
manifest_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
PY
}

ensure_local_keys() {
  if [ -f "$private_key_path" ] && [ -f "$public_key_path" ]; then
    return 0
  fi

  if [ "${OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS:-0}" != "1" ]; then
    die "release signing keys are not configured; set OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS=1 for local preview or provide signing secrets in CI"
  fi

  mkdir -p "$(dirname "$private_key_path")"
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$private_key_path" >/dev/null 2>&1
  openssl pkey -in "$private_key_path" -pubout -out "$public_key_path" >/dev/null 2>&1
  generated_keys=1
}

decode_env_key() {
  local value="$1"
  local path="$2"
  local label="$3"
  python3 - "$value" "$path" "$label" <<'PY'
import base64
import binascii
import sys
from pathlib import Path

value = sys.argv[1]
path = Path(sys.argv[2])
label = sys.argv[3]

try:
    key_bytes = base64.b64decode(value, validate=True)
except binascii.Error as exc:
    print(f"FAILED: invalid base64 release signing {label}: {exc}", file=sys.stderr)
    sys.exit(1)

path.write_bytes(key_bytes)
PY
}

load_private_key_for_sign() {
  if [ -n "${OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64:-}" ] && [ -n "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64:-}" ]; then
    local temp_dir
    temp_dir="$(mktemp -d)"
    private_key_path="$temp_dir/private.pem"
    public_key_path="$temp_dir/public.pem"
    decode_env_key "${OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64}" "$private_key_path" "private-key"
    decode_env_key "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64}" "$public_key_path" "public-key"
    return 0
  fi

  if [ -f "$private_key_path" ] && [ -f "$public_key_path" ]; then
    return 0
  fi

  ensure_local_keys
}

load_public_key_for_verify() {
  if [ -n "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64:-}" ]; then
    local temp_dir
    temp_dir="$(mktemp -d)"
    public_key_path="$temp_dir/public.pem"
    decode_env_key "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64}" "$public_key_path" "public-key"
    return 0
  fi

  if [ -f "$public_key_path" ]; then
    return 0
  fi

  if [ "${OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS:-0}" = "1" ]; then
    ensure_local_keys
    return 0
  fi

  die "release signing public key is not configured"
}

sign_manifest() {
  openssl dgst -sha256 -sign "$private_key_path" -out "$signature_path" "$manifest_path" >/dev/null 2>&1
}

verify_manifest() {
  [ -f "$manifest_path" ] || die "missing release manifest: $manifest_path"
  [ -f "$signature_path" ] || die "missing release manifest signature: $signature_path"
  openssl dgst -sha256 -verify "$public_key_path" -signature "$signature_path" "$manifest_path" >/dev/null 2>&1 \
    || die "release manifest signature verification failed"
}

case "$command" in
  prepare)
    [ -n "$artifact_dir" ] || die "artifact directory required"
    [ -n "$binary_name" ] || die "binary name required"
    create_manifest
    load_private_key_for_sign
    sign_manifest
    verify_manifest
    echo "Prepared signed release manifest:"
    echo "  manifest: $manifest_path"
    echo "  signature: $signature_path"
    if [ "$generated_keys" = "1" ]; then
      echo "  signing: local generated preview keys"
    fi
    ;;
  verify)
    [ -n "$artifact_dir" ] || die "artifact directory required"
    [ -n "$binary_name" ] || die "binary name required"
    [ -f "$manifest_path" ] || die "missing release manifest: $manifest_path"
    [ -f "$signature_path" ] || die "missing release manifest signature: $signature_path"
    load_public_key_for_verify
    verify_manifest
    echo "Verified signed release manifest:"
    echo "  manifest: $manifest_path"
    echo "  signature: $signature_path"
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
