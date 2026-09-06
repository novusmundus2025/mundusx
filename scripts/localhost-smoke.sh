#!/usr/bin/env bash
set -euo pipefail

dashboard_url="${DASHBOARD_URL:-http://127.0.0.1:3002}"
control_plane_url="${CONTROL_PLANE_URL:-http://127.0.0.1:8787}"
release_base_url="${RELEASE_BASE_URL:-http://127.0.0.1:8788/releases/latest/download}"
install_dir="${INSTALL_DIR:-/private/tmp/opengpu-local-smoke-install}"
skip_control_plane="${SMOKE_SKIP_CONTROL_PLANE:-0}"
skip_release="${SMOKE_SKIP_RELEASE:-0}"
skip_install="${SMOKE_SKIP_INSTALL:-0}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
local_release_preview_helper="${LOCAL_RELEASE_PREVIEW_HELPER:-$repo_root/scripts/local-release-preview.sh}"
docs_site_dir="${SMOKE_DOCS_SITE_DIR:-$repo_root/dist/public-docs-site}"
docs_server_pid=""
dashboard_check_mode="http"

cleanup() {
  if [ -n "$docs_server_pid" ] && kill -0 "$docs_server_pid" 2>/dev/null; then
    kill "$docs_server_pid" >/dev/null 2>&1 || true
    wait "$docs_server_pid" 2>/dev/null || true
  fi
}

trap cleanup EXIT

check_contains() {
  local url="$1"
  local needle="$2"
  local label="$3"

  if ! curl -fsSL --max-time 3 "$url" | rg -q "$needle"; then
    echo "FAILED: $label"
    echo "  url: $url"
    echo "  missing: $needle"
    exit 1
  fi
  echo "OK: $label"
}

check_file_contains() {
  local file_path="$1"
  local needle="$2"
  local label="$3"

  if ! rg -q "$needle" "$file_path"; then
    echo "FAILED: $label"
    echo "  file: $file_path"
    echo "  missing: $needle"
    exit 1
  fi
  echo "OK: $label"
}

dashboard_file_for_url() {
  local url="$1"
  local path_part

  path_part="${url#"$dashboard_url"}"
  path_part="${path_part#/}"

  case "$path_part" in
    ""|"docs")
      printf '%s\n' "$docs_site_dir/index.html"
      ;;
    "install")
      printf '%s\n' "$docs_site_dir/public/install/index.html"
      ;;
    "install.json")
      printf '%s\n' "$docs_site_dir/public/install.json"
      ;;
    "public/install")
      printf '%s\n' "$docs_site_dir/public/install/index.html"
      ;;
    "public/install.json")
      printf '%s\n' "$docs_site_dir/public/install.json"
      ;;
    "public/docs")
      printf '%s\n' "$docs_site_dir/public/docs/index.html"
      ;;
    "public/docs/install")
      printf '%s\n' "$docs_site_dir/public/docs/install/index.html"
      ;;
    *)
      echo "unsupported dashboard preview path: $url" >&2
      exit 1
      ;;
  esac
}

check_dashboard_contains() {
  local url="$1"
  local needle="$2"
  local label="$3"
  local static_needle="${4:-$needle}"

  if [ "$dashboard_check_mode" = "http" ]; then
    check_contains "$url" "$needle" "$label"
    return
  fi

  check_file_contains "$(dashboard_file_for_url "$url")" "$static_needle" "$label"
}

parse_http_host_port() {
  local url="$1"
  local hostport pathless host port

  pathless="${url#http://}"
  if [ "$pathless" = "$url" ]; then
    echo "unsupported dashboard url: $url" >&2
    exit 1
  fi

  hostport="${pathless%%/*}"
  host="${hostport%%:*}"
  port="${hostport##*:}"

  if [ -z "$host" ] || [ -z "$port" ] || [ "$host" = "$port" ]; then
    echo "dashboard url must include host and port: $url" >&2
    exit 1
  fi

  printf '%s %s\n' "$host" "$port"
}

ensure_dashboard_preview() {
  local host port server_log_file

  if curl -fsSL --max-time 3 "${dashboard_url}/install" >/dev/null 2>&1; then
    return
  fi

  read -r host port <<<"$(parse_http_host_port "$dashboard_url")"

  echo "Dashboard preview not detected at ${dashboard_url}; building static docs site..."
  npm run build:docs-site >/dev/null

  server_log_file="$repo_root/.opengpu/local-docs-preview.log"
  mkdir -p "$(dirname "$server_log_file")"
  nohup python3 -m http.server "$port" --bind "$host" --directory "$docs_site_dir" \
    >"$server_log_file" 2>&1 &
  docs_server_pid="$!"

  for _ in $(seq 1 20); do
    if curl -fsSL --max-time 3 "${dashboard_url}/install" >/dev/null 2>&1; then
      echo "Local docs preview is live."
      return
    fi
    sleep 0.5
  done

  if rg -q "Operation not permitted|PermissionError" "$server_log_file"; then
    dashboard_check_mode="static"
    echo "Local docs preview server could not bind; falling back to static file verification."
    return
  fi

  echo "FAILED: local docs preview did not become ready"
  echo "  url: ${dashboard_url}/install"
  echo "  log: $server_log_file"
  exit 1
}

echo "Checking dashboard pages..."
ensure_dashboard_preview
check_dashboard_contains "${dashboard_url}/install" "Install MundusX on your Mac" "install page hero"
check_dashboard_contains "${dashboard_url}/install" "Fetching ./install.json" "install manifest loading state" "Fetching ../install.json"
check_dashboard_contains "${dashboard_url}/install" "./install.json" "install manifest endpoint reference" "../install.json"
check_dashboard_contains "${dashboard_url}/install.json" "\"kind\": \"install-manifest\"" "install manifest kind"
check_dashboard_contains \
  "${dashboard_url}/install.json" \
  "\"install_command\": \"RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh\"" \
  "install manifest command" \
  "\"release_base_url\": \"https://github.com/mundusx/mundusx/releases/latest/download\""
check_dashboard_contains "${dashboard_url}/public/install" "Install MundusX on your Mac" "public install mirror hero"
check_dashboard_contains "${dashboard_url}/public/install" "Fetching ../install.json" "public install mirror loading state"
check_dashboard_contains "${dashboard_url}/public/install.json" "\"kind\": \"install-manifest\"" "public install mirror manifest kind"
check_dashboard_contains "${dashboard_url}/docs" "MundusX Docs" "docs home"
check_dashboard_contains "${dashboard_url}/public/docs" "MundusX Docs" "public docs mirror home"
check_dashboard_contains "${dashboard_url}/public/docs" "local layout" "public docs mirror badge" "Public Endpoint Mirror"
check_dashboard_contains "${dashboard_url}/public/docs/install" "Canonical command" "public docs mirror install page"

echo "Checking control-plane root..."
if [ "$skip_control_plane" = "1" ]; then
  echo "Skipping control-plane checks."
else
  check_contains "${control_plane_url}/" "MundusX Control Plane" "control plane root"
  check_contains "${control_plane_url}/" "Local operator view" "control plane hero"
fi

if [ "$skip_release" = "1" ]; then
  echo "Skipping release preview and installer checks."
  echo "Smoke test complete."
  exit 0
fi

echo "Preparing local release preview..."
"$local_release_preview_helper" up
"$local_release_preview_helper" verify

echo "Checking release source..."
check_contains "${release_base_url}/" "MundusX Local Release Preview" "release landing page"
check_contains "${release_base_url}/" "localhost only" "release landing page badge"
check_contains "${release_base_url}/" "Manifest loaded from /release-manifest.json" "release landing page manifest state"
check_contains "${release_base_url}/release-manifest.json" "\"artifact_kind\": \"release-binary\"" "release manifest kind"
check_contains "${release_base_url}/opengpu-aarch64-apple-darwin.sha256" "opengpu-aarch64-apple-darwin" "release checksum"

echo "Checking release monitor report..."
report="$("$repo_root/scripts/release-monitor-report.sh" \
  "$repo_root/.opengpu/local-release-preview/releases/latest/download" \
  "opengpu-aarch64-apple-darwin")"
python3 - <<'PY' "$report"
import json
import sys

report = json.loads(sys.argv[1])
assert report["manifest_signature_verified"] is True
assert report["checksum_matches_manifest"] is True
PY

if [ "$skip_install" = "1" ]; then
  echo "Skipping installer check."
else
  echo "Running installer..."
  rm -rf "$install_dir"
  mkdir -p "$install_dir"
  INSTALL_DIR="$install_dir" RELEASE_BASE_URL="$release_base_url" bash "$repo_root/install.sh"
  "$install_dir/opengpu" --version
fi

echo "Smoke test complete."
