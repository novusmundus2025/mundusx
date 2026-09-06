#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${1:-}"
binary_name="${2:-}"

usage() {
  cat <<'EOF'
Usage: scripts/release-monitor-report.sh <artifact-dir> <binary-name>

Prints a machine-readable JSON report for a release artifact directory,
including binary/checksum presence and signed manifest verification state.
EOF
}

die() {
  echo "FAILED: $*" >&2
  exit 1
}

if [ -z "$artifact_dir" ] || [ -z "$binary_name" ]; then
  usage >&2
  exit 1
fi

binary_path="$artifact_dir/$binary_name"
checksum_path="$binary_path.sha256"
manifest_path="$artifact_dir/release-manifest.json"
signature_path="$artifact_dir/release-manifest.json.sig"

binary_present=false
checksum_present=false
manifest_present=false
signature_present=false
checksum_matches_manifest=false
manifest_signature_verified=false

[ -f "$binary_path" ] && binary_present=true
[ -f "$checksum_path" ] && checksum_present=true
[ -f "$manifest_path" ] && manifest_present=true
[ -f "$signature_path" ] && signature_present=true

if ! $binary_present || ! $checksum_present || ! $manifest_present || ! $signature_present; then
  die "release surface is incomplete"
fi

if command -v shasum >/dev/null 2>&1; then
  checksum="$(shasum -a 256 "$binary_path" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  checksum="$(sha256sum "$binary_path" | awk '{print $1}')"
else
  die "checksum tooling not found (need shasum or sha256sum)"
fi

manifest_fields="$(
  python3 - "$manifest_path" <<'PY'
import json
import sys

manifest = json.load(open(sys.argv[1]))
print(manifest.get("artifact_kind", ""))
print(manifest.get("binary_name", ""))
print(manifest.get("checksum_sha256", ""))
print(manifest.get("generated_at", ""))
print(manifest.get("tag", ""))
print(manifest.get("version", ""))
PY
)"

artifact_kind="$(printf '%s\n' "$manifest_fields" | sed -n '1p')"
manifest_binary_name="$(printf '%s\n' "$manifest_fields" | sed -n '2p')"
manifest_checksum="$(printf '%s\n' "$manifest_fields" | sed -n '3p')"
generated_at="$(printf '%s\n' "$manifest_fields" | sed -n '4p')"
tag="$(printf '%s\n' "$manifest_fields" | sed -n '5p')"
version="$(printf '%s\n' "$manifest_fields" | sed -n '6p')"

if [ "$manifest_binary_name" = "$binary_name" ] && [ "$manifest_checksum" = "$checksum" ]; then
  checksum_matches_manifest=true
fi

if OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS=1 \
  "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/release-signing.sh" verify "$artifact_dir" "$binary_name" >/dev/null 2>&1; then
  manifest_signature_verified=true
fi

python3 - <<'PY' \
  "$artifact_kind" \
  "$binary_name" \
  "$checksum" \
  "$generated_at" \
  "$tag" \
  "$version" \
  "$binary_present" \
  "$checksum_present" \
  "$manifest_present" \
  "$signature_present" \
  "$checksum_matches_manifest" \
  "$manifest_signature_verified"
import json
import sys

def as_bool(value: str) -> bool:
    return value == "true"

payload = {
    "artifact_kind": sys.argv[1],
    "binary_name": sys.argv[2],
    "checksum_sha256": sys.argv[3],
    "generated_at": sys.argv[4],
    "tag": sys.argv[5],
    "version": sys.argv[6],
    "binary_present": as_bool(sys.argv[7]),
    "checksum_present": as_bool(sys.argv[8]),
    "manifest_present": as_bool(sys.argv[9]),
    "signature_present": as_bool(sys.argv[10]),
    "checksum_matches_manifest": as_bool(sys.argv[11]),
    "manifest_signature_verified": as_bool(sys.argv[12]),
}
print(json.dumps(payload, indent=2, sort_keys=True))
PY
