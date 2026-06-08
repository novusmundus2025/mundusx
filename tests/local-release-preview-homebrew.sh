#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
preview_root="$(mktemp -d)"
trap 'OPENGPU_LOCAL_RELEASE_PREVIEW_DIR="$preview_root" "$repo_root/scripts/local-release-preview.sh" clean >/dev/null 2>&1 || true; rm -rf "$preview_root"' EXIT

OPENGPU_LOCAL_RELEASE_PREVIEW_DIR="$preview_root" \
  "$repo_root/scripts/local-release-preview.sh" build >/dev/null

formula_path="$preview_root/releases/latest/download/homebrew/opengpu.rb"
index_path="$preview_root/releases/latest/download/index.html"

[[ -f "$formula_path" ]] || {
  echo "expected local release preview to create a Homebrew formula" >&2
  exit 1
}

grep -F 'class Opengpu < Formula' "$formula_path" >/dev/null
grep -F 'http://127.0.0.1:8788/releases/latest/download/opengpu-' "$formula_path" >/dev/null
grep -F 'Homebrew formula' "$index_path" >/dev/null
grep -F './homebrew/opengpu.rb' "$index_path" >/dev/null

echo "Local release preview Homebrew surface verified."
