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
assert_contains "agent_binary_name: opengpu-node-agent-x86_64-pc-windows-msvc.exe"
assert_contains "agent_executable_name: opengpu-node-agent.exe"
assert_contains "tray_binary_name: mundusx-tray-x86_64-pc-windows-msvc.exe"
assert_contains "cargo build --release --manifest-path apps/tray/Cargo.toml"
assert_contains "setup_binary_name: MundusX-Setup.exe"
assert_contains "cargo build --release --manifest-path apps/windows-installer/Cargo.toml"
assert_contains "runtime_binary_name: llama-runtime-x86_64-pc-windows-msvc-cuda.zip"
assert_contains "package-windows-llama-runtime.ps1 -Tag b9856 -CudaVersion 12.4"
assert_contains "cargo build --release --manifest-path agents/node/Cargo.toml --target \${{ matrix.target }}"
assert_contains "cp target/\${{ matrix.target }}/release/\${{ matrix.executable_name }} \${{ matrix.binary_name }}"
assert_contains "cp target/\${{ matrix.target }}/release/\${{ matrix.agent_executable_name }} \${{ matrix.agent_binary_name }}"
assert_contains "sha256sum \${{ matrix.binary_name }} > \${{ matrix.binary_name }}.sha256"
assert_contains "sha256sum \${{ matrix.agent_binary_name }} > \${{ matrix.agent_binary_name }}.sha256"
assert_contains "sha256sum -c \${{ matrix.binary_name }}.sha256"
assert_contains "sha256sum -c \${{ matrix.agent_binary_name }}.sha256"
assert_contains "./scripts/verify-release-packaging.sh . \${{ matrix.binary_name }} \${{ matrix.agent_binary_name }}"
assert_contains "./scripts/release-signing.sh prepare . \${{ matrix.binary_name }} \${{ github.ref_name }} \${{ github.ref_name }}"

echo "release CLI workflow publishes CLI, node-agent, Windows tray, and clickable setup assets"
