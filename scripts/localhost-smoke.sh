#!/usr/bin/env bash
set -euo pipefail

dashboard_url="${DASHBOARD_URL:-http://127.0.0.1:3002}"
control_plane_url="${CONTROL_PLANE_URL:-http://127.0.0.1:8787}"
release_base_url="${RELEASE_BASE_URL:-http://127.0.0.1:8788/releases/latest/download}"
install_dir="${INSTALL_DIR:-/private/tmp/opengpu-local-smoke-install}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
local_release_preview_helper="${LOCAL_RELEASE_PREVIEW_HELPER:-$repo_root/scripts/local-release-preview.sh}"

check_contains() {
  local url="$1"
  local needle="$2"
  local label="$3"

  if ! curl -fsS --max-time 3 "$url" | rg -q "$needle"; then
    echo "FAILED: $label"
    echo "  url: $url"
    echo "  missing: $needle"
    exit 1
  fi
  echo "OK: $label"
}

echo "Checking dashboard pages..."
check_contains "${dashboard_url}/install" "Install OpenGPU on your Mac" "install page hero"
check_contains "${dashboard_url}/install" "Fetching /install.json" "install manifest loading state"
check_contains "${dashboard_url}/install" "/install.json" "install manifest endpoint reference"
check_contains "${dashboard_url}/install.json" "\"kind\": \"install-manifest\"" "install manifest kind"
check_contains "${dashboard_url}/install.json" "\"install_command\": \"RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh\"" "install manifest command"
check_contains "${dashboard_url}/docs" "OpenGPU Docs" "docs home"

echo "Checking control-plane root..."
check_contains "${control_plane_url}/" "OpenGPU Control Plane" "control plane root"
check_contains "${control_plane_url}/" "Local operator view" "control plane hero"

echo "Preparing local release preview..."
"$local_release_preview_helper" up
"$local_release_preview_helper" verify

echo "Checking release source..."
check_contains "${release_base_url}/" "OpenGPU Local Release Preview" "release landing page"
check_contains "${release_base_url}/" "localhost only" "release landing page badge"
check_contains "${release_base_url}/" "Manifest loaded from /release-manifest.json" "release landing page manifest state"
check_contains "${release_base_url}/release-manifest.json" "\"artifact_kind\": \"release-binary\"" "release manifest kind"
check_contains "${release_base_url}/opengpu-aarch64-apple-darwin.sha256" "opengpu-aarch64-apple-darwin" "release checksum"

echo "Running installer..."
rm -rf "$install_dir"
mkdir -p "$install_dir"
INSTALL_DIR="$install_dir" RELEASE_BASE_URL="$release_base_url" bash "$repo_root/install.sh"
"$install_dir/opengpu" --version

echo "Smoke test complete."
