#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${1:-}"
binary_name="${2:-}"
agent_binary_name="${3:-}"

usage() {
  cat <<'EOF'
Usage: scripts/verify-release-packaging.sh <artifact-dir> <binary-name> [node-agent-binary-name]

Verifies that a release artifact directory contains the expected binary,
checksum file, optional node-agent binary/checksum, and optional preview
landing page, and that the checksums match.
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
agent_binary_path=""
agent_checksum_path=""
index_path="$artifact_dir/index.html"

[ -f "$binary_path" ] || die "missing release binary: $binary_path"
[ -x "$binary_path" ] || die "release binary is not executable: $binary_path"
[ -f "$checksum_path" ] || die "missing checksum file: $checksum_path"

if [ -n "$agent_binary_name" ]; then
  agent_binary_path="$artifact_dir/$agent_binary_name"
  agent_checksum_path="$agent_binary_path.sha256"
  [ -f "$agent_binary_path" ] || die "missing node-agent binary: $agent_binary_path"
  [ -x "$agent_binary_path" ] || die "node-agent binary is not executable: $agent_binary_path"
  [ -f "$agent_checksum_path" ] || die "missing node-agent checksum file: $agent_checksum_path"
fi

if command -v shasum >/dev/null 2>&1; then
  (cd "$artifact_dir" && shasum -a 256 -c "$(basename "$checksum_path")")
  if [ -n "$agent_checksum_path" ]; then
    (cd "$artifact_dir" && shasum -a 256 -c "$(basename "$agent_checksum_path")")
  fi
elif command -v sha256sum >/dev/null 2>&1; then
  (cd "$artifact_dir" && sha256sum -c "$(basename "$checksum_path")")
  if [ -n "$agent_checksum_path" ]; then
    (cd "$artifact_dir" && sha256sum -c "$(basename "$agent_checksum_path")")
  fi
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
if [ -n "$agent_binary_name" ]; then
  echo "  agent: $agent_binary_name"
fi
