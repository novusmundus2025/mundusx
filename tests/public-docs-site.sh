#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
out_dir="$(mktemp -d)"
trap 'rm -rf "$out_dir"' EXIT

node "$repo_root/scripts/build-public-docs-site.mjs" "$out_dir"

require_file() {
  local path="$1"
  if [[ ! -f "$out_dir/$path" ]]; then
    echo "missing generated file: $path" >&2
    exit 1
  fi
}

require_line() {
  local path="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$out_dir/$path"; then
    echo "missing pattern '$pattern' in generated $path" >&2
    exit 1
  fi
}

require_repo_line() {
  local path="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$repo_root/$path"; then
    echo "missing pattern '$pattern' in $path" >&2
    exit 1
  fi
}

require_file "index.html"
require_file "install/index.html"
require_file "device-identity/index.html"
require_file "public/docs/index.html"
require_file "public/docs/install/index.html"
require_file "public/install/index.html"
require_file "public/install.json"
require_file "public/install.sh"
require_file "public/install.ps1"
require_file "public/release/index.html"
require_file "public/release.json"

require_line "index.html" "MundusX Docs"
require_line "index.html" "Install MundusX"
require_line "install/index.html" "Canonical command"
require_line "install/index.html" "RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh"
require_line "device-identity/index.html" "Device Identity Lifecycle"
require_line "device-identity/index.html" "non-exportable device keys"
require_line "public/docs/index.html" "Public Endpoint Mirror"
require_line "public/docs/install/index.html" "Canonical command"
require_line "public/install/index.html" "Install MundusX"
require_line "public/install/index.html" "Fetching ../install.json"
require_line "public/install.json" "\"kind\": \"install-manifest\""
require_line "public/install.json" "\"release_base_url\": \"https://github.com/mundusx/mundusx/releases/latest/download\""
require_line "public/install.sh" "REPO=\"mundusx/mundusx\""
require_line "public/install.ps1" "x86_64-pc-windows-msvc"
require_line "public/release/index.html" "Public Release Mirror"
require_line "public/release/index.html" "Fetching ../release.json"
require_line "public/release.json" "\"kind\": \"release-channel\""
require_line "public/release.json" "\"release_notes_url\": \"https://github.com/mundusx/mundusx/releases/latest\""

require_repo_line ".github/workflows/public-docs-site.yml" "name: Deploy Public Docs Site"
require_repo_line ".github/workflows/public-docs-site.yml" "uses: actions/deploy-pages@v4"

echo "Public docs site build and deployment wiring verified."
