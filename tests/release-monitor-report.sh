#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

artifact_dir="$tmp_dir/artifacts"
binary_name="opengpu-aarch64-apple-darwin"
binary_path="$artifact_dir/$binary_name"

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

OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS=1 \
  "$repo_root/scripts/release-signing.sh" prepare "$artifact_dir" "$binary_name" "cli-v0.1.0" "0.1.0" >/dev/null

report="$("$repo_root/scripts/release-monitor-report.sh" "$artifact_dir" "$binary_name")"

python3 - <<'PY' "$report" "$checksum" "$binary_name"
import json
import sys

report = json.loads(sys.argv[1])
checksum = sys.argv[2]
binary_name = sys.argv[3]

assert report["artifact_kind"] == "release-binary"
assert report["binary_name"] == binary_name
assert report["checksum_sha256"] == checksum
assert report["checksum_matches_manifest"] is True
assert report["manifest_signature_verified"] is True
assert report["binary_present"] is True
assert report["checksum_present"] is True
assert report["manifest_present"] is True
assert report["signature_present"] is True
assert report["version"] == "0.1.0"
assert report["tag"] == "cli-v0.1.0"
assert report["generated_at"]
PY

echo "Release monitor report verified."
