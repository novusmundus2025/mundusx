#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

artifact_dir="$tmp_dir/artifacts"
binary_name="opengpu-aarch64-apple-darwin"
binary_path="$artifact_dir/$binary_name"
formula_path="$artifact_dir/homebrew/opengpu.rb"
binary_url="https://github.com/mundusx/mundusx/releases/download/cli-v0.1.0/$binary_name"

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

node "$repo_root/scripts/render-homebrew-formula.mjs" \
  "$artifact_dir/release-manifest.json" \
  "$binary_url" \
  "$formula_path"

[[ -f "$formula_path" ]] || {
  echo "expected formula file to be created" >&2
  exit 1
}

grep -F 'class Opengpu < Formula' "$formula_path" >/dev/null
grep -F 'desc "MundusX contributor CLI"' "$formula_path" >/dev/null
grep -F "url \"$binary_url\"" "$formula_path" >/dev/null
grep -F "sha256 \"$checksum\"" "$formula_path" >/dev/null
grep -F 'version "0.1.0"' "$formula_path" >/dev/null
grep -F 'bin.install "opengpu-aarch64-apple-darwin" => "opengpu"' "$formula_path" >/dev/null

echo "Homebrew formula renderer verified."
