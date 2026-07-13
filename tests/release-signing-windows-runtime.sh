#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

binary="opengpu-x86_64-pc-windows-msvc.exe"
agent="opengpu-node-agent-x86_64-pc-windows-msvc.exe"
tray="mundusx-tray-x86_64-pc-windows-msvc.exe"
runtime="llama-runtime-x86_64-pc-windows-msvc-cuda.zip"
for asset in "$binary" "$agent" "$tray" "$runtime"; do
  printf 'fixture for %s\n' "$asset" >"$tmp_dir/$asset"
  sha256sum "$tmp_dir/$asset" >"$tmp_dir/$asset.sha256"
done

OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS=1 \
  "$repo_root/scripts/release-signing.sh" prepare "$tmp_dir" "$binary" cli-v0.1.0 0.1.0 >/dev/null

python - "$tmp_dir/release-manifest.json" "$tray" "$runtime" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text())
tray_name = sys.argv[2]
runtime_name = sys.argv[3]
assets = {asset["name"]: asset for asset in manifest["assets"]}
assert assets[tray_name]["kind"] == "windows-tray-binary"
runtime = manifest["runtime_assets"][0]
assert runtime["name"] == runtime_name
assert runtime["install_as"] == "runtimes/llama"
assert runtime["kind"] == "llama-cpp-cuda-runtime-bundle"
assert len(runtime["checksum_sha256"]) == 64
PY

echo "Windows tray and CUDA runtime are present in the signed release manifest"
