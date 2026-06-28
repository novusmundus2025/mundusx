#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

workflow=".github/workflows/release-cli.yml"

assert_contains() {
  local text="$1"

  if ! rg -F -q "$text" "$workflow"; then
    echo "expected $workflow to contain: $text" >&2
    exit 1
  fi
}

assert_contains "os: windows-latest"
assert_contains "target: x86_64-pc-windows-msvc"
assert_contains "binary_name: opengpu-x86_64-pc-windows-msvc.exe"
assert_contains "executable_name: opengpu.exe"
assert_contains "cp target/\${{ matrix.target }}/release/\${{ matrix.executable_name }} \${{ matrix.binary_name }}"
assert_contains "sha256sum \${{ matrix.binary_name }} > \${{ matrix.binary_name }}.sha256"
assert_contains "sha256sum -c \${{ matrix.binary_name }}.sha256"
assert_contains "./scripts/release-signing.sh prepare . \${{ matrix.binary_name }} \${{ github.ref_name }} \${{ github.ref_name }}"

echo "release CLI workflow publishes the Windows MSVC asset"
