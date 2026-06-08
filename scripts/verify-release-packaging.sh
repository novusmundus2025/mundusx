#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${1:-}"
binary_name="${2:-}"

usage() {
  cat <<'EOF'
Usage: scripts/verify-release-packaging.sh <artifact-dir> <binary-name>

Verifies that a release artifact directory contains the expected binary,
checksum file, and optional preview landing page, and that the checksum matches.
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
index_path="$artifact_dir/index.html"

[ -f "$binary_path" ] || die "missing release binary: $binary_path"
[ -x "$binary_path" ] || die "release binary is not executable: $binary_path"
[ -f "$checksum_path" ] || die "missing checksum file: $checksum_path"

if command -v shasum >/dev/null 2>&1; then
  (cd "$artifact_dir" && shasum -a 256 -c "$(basename "$checksum_path")")
elif command -v sha256sum >/dev/null 2>&1; then
  (cd "$artifact_dir" && sha256sum -c "$(basename "$checksum_path")")
else
  die "checksum tooling not found (need shasum or sha256sum)"
fi

if [ -f "$index_path" ]; then
  rg -q "MundusX Local Release Preview" "$index_path" || die "release landing page title missing"
  rg -q "localhost only" "$index_path" || die "release landing page badge missing"
fi

echo "Verified release packaging:"
echo "  dir:   $artifact_dir"
echo "  asset: $binary_name"
