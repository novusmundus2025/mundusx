#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

legacy_workflow=".github/workflows/release-cli.yml"
macos_workflow=".github/workflows/release-cli-macos.yml"
windows_workflow=".github/workflows/release-cli-windows.yml"
linux_workflow=".github/workflows/release-cli-linux.yml"

assert_contains() {
  local workflow="$1"
  local text="$2"

  if ! rg -F -q "$text" "$workflow"; then
    echo "expected $workflow to contain: $text" >&2
    exit 1
  fi
}

assert_legacy_contains() {
  local text="$1"

  assert_contains "$legacy_workflow" "$text"
}

assert_legacy_contains "os: windows-latest"
assert_legacy_contains "workflow_dispatch:"
assert_legacy_contains "release_tag:"
assert_legacy_contains "channel: macos"
assert_legacy_contains "channel: windows"

assert_contains "$macos_workflow" "name: Release CLI macOS"
assert_contains "$macos_workflow" "cli-macos-v*"
assert_contains "$macos_workflow" "runs-on: macos-14"
assert_contains "$macos_workflow" "TARGET: aarch64-apple-darwin"
assert_contains "$macos_workflow" "BINARY_NAME: opengpu-aarch64-apple-darwin"
assert_contains "$macos_workflow" "AGENT_BINARY_NAME: opengpu-node-agent-aarch64-apple-darwin"
assert_contains "$macos_workflow" "cargo build --release --manifest-path apps/cli/Cargo.toml --target \${{ env.TARGET }}"
assert_contains "$macos_workflow" "cargo build --release --manifest-path agents/node/Cargo.toml --target \${{ env.TARGET }}"
assert_contains "$macos_workflow" "tag_name: \${{ env.RELEASE_TAG }}"
assert_contains "$macos_workflow" "name: MundusX macOS Apple Silicon CLI \${{ env.RELEASE_TAG }}"

assert_contains "$windows_workflow" "name: Release CLI Windows"
assert_contains "$windows_workflow" "cli-windows-v*"
assert_contains "$windows_workflow" "runs-on: windows-latest"
assert_contains "$windows_workflow" "TARGET: x86_64-pc-windows-msvc"
assert_contains "$windows_workflow" "BINARY_NAME: opengpu-x86_64-pc-windows-msvc.exe"
assert_contains "$windows_workflow" "AGENT_BINARY_NAME: opengpu-node-agent-x86_64-pc-windows-msvc.exe"
assert_contains "$windows_workflow" "TRAY_BINARY_NAME: mundusx-tray-x86_64-pc-windows-msvc.exe"
assert_contains "$windows_workflow" "SETUP_BINARY_NAME: MundusX-Setup.exe"
assert_contains "$windows_workflow" "RUNTIME_BINARY_NAME: llama-runtime-x86_64-pc-windows-msvc-cuda.zip"
assert_contains "$windows_workflow" "VULKAN_RUNTIME_BINARY_NAME: llama-runtime-x86_64-pc-windows-msvc-vulkan.zip"
assert_contains "$windows_workflow" "package-windows-llama-runtime.ps1 -Tag b9856 -CudaVersion 12.4"
assert_contains "$windows_workflow" "package-windows-vulkan-runtime.ps1 -Tag b9856"
assert_contains "$windows_workflow" "cargo build --release --manifest-path apps/tray/Cargo.toml --target \${{ env.TARGET }}"
assert_contains "$windows_workflow" "cargo build --release --manifest-path apps/windows-installer/Cargo.toml --target \${{ env.TARGET }}"
assert_contains "$windows_workflow" "tag_name: \${{ env.RELEASE_TAG }}"
assert_contains "$windows_workflow" "name: MundusX Windows x86_64 CLI \${{ env.RELEASE_TAG }}"

assert_contains "$linux_workflow" "name: Release CLI Linux"
assert_contains "$linux_workflow" "cli-linux-v*"
assert_contains "$linux_workflow" "target: aarch64-unknown-linux-gnu"
assert_contains "$linux_workflow" "runner: ubuntu-24.04-arm"
assert_contains "$linux_workflow" "BINARY_NAME: opengpu-\${{ matrix.target }}"
assert_contains "$linux_workflow" "AGENT_BINARY_NAME: opengpu-node-agent-\${{ matrix.target }}"
assert_contains "$linux_workflow" 'mv release-manifest.json "release-manifest-${TARGET}.json"'
assert_contains "$linux_workflow" "name: MundusX Linux CLI \${{ env.RELEASE_TAG }}"

echo "release CLI workflows publish separate Linux, macOS, and Windows CLI releases"
