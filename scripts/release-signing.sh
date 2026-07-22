#!/usr/bin/env bash
set -euo pipefail

command="${1:-}"
artifact_dir="${2:-}"
binary_name="${3:-}"
tag_name="${4:-}"
version="${5:-}"

usage() {
  cat <<'EOF'
Usage: scripts/release-signing.sh <prepare|verify> <artifact-dir> <binary-name> [tag] [version]

Commands:
  prepare  Create a signed release manifest for the artifact directory.
  verify   Verify the release manifest signature for the artifact directory.
EOF
}

die() {
  echo "FAILED: $*" >&2
  exit 1
}

if [ -z "$command" ] || [ -z "$artifact_dir" ] || [ -z "$binary_name" ]; then
  usage >&2
  exit 1
fi

manifest_path="$artifact_dir/release-manifest.json"
signature_path="$artifact_dir/release-manifest.json.sig"
private_key_path="${artifact_dir}/.signing/private.pem"
public_key_path="${artifact_dir}/.signing/public.pem"
generated_keys=0

python_cmd() {
  if command -v python3 >/dev/null 2>&1; then
    printf 'python3\n'
  elif command -v python >/dev/null 2>&1; then
    printf 'python\n'
  else
    die "python3 or python is required"
  fi
}

normalize_text() {
  local value="$1"
  if [ -n "$value" ]; then
    printf '%s' "$value"
  else
    printf 'local-preview'
  fi
}

checksum_for_binary() {
  local name="${1:-$binary_name}"
  local checksum_path="$artifact_dir/$name.sha256"
  [ -f "$checksum_path" ] || die "missing checksum file: $checksum_path"

  awk '{print $1}' "$checksum_path" | head -n 1
}

node_agent_name_for_binary() {
  case "$binary_name" in
    opengpu-*) printf 'opengpu-node-agent-%s\n' "${binary_name#opengpu-}" ;;
    *) printf '' ;;
  esac
}

create_manifest() {
  local checksum tag_text version_text generated_at agent_name agent_checksum agent_install_as tray_name tray_checksum runtime_name runtime_checksum vulkan_runtime_name vulkan_runtime_checksum mac_pkg_name mac_pkg_checksum
  checksum="$(checksum_for_binary "$binary_name")"
  tag_text="$(normalize_text "$tag_name")"
  version_text="$(normalize_text "$version")"
  agent_name="$(node_agent_name_for_binary)"
  agent_checksum=""
  agent_install_as=""
  if [ -n "$agent_name" ] && [ -f "$artifact_dir/$agent_name" ]; then
    agent_checksum="$(checksum_for_binary "$agent_name")"
    case "$agent_name" in
      *.exe) agent_install_as="opengpu-node-agent.exe" ;;
      *) agent_install_as="opengpu-node-agent" ;;
    esac
  fi
  tray_name=""
  tray_checksum=""
  case "$binary_name" in
    opengpu-*.exe) tray_name="mundusx-tray-${binary_name#opengpu-}" ;;
  esac
  if [ -n "$tray_name" ] && [ -f "$artifact_dir/$tray_name" ]; then
    tray_checksum="$(checksum_for_binary "$tray_name")"
  fi
  runtime_name=""
  runtime_checksum=""
  case "$binary_name" in
    opengpu-*.exe) runtime_name="llama-runtime-x86_64-pc-windows-msvc-cuda.zip" ;;
  esac
  if [ -n "$runtime_name" ] && [ -f "$artifact_dir/$runtime_name" ]; then
    runtime_checksum="$(checksum_for_binary "$runtime_name")"
  fi
  vulkan_runtime_name=""
  vulkan_runtime_checksum=""
  case "$binary_name" in
    opengpu-*.exe) vulkan_runtime_name="llama-runtime-x86_64-pc-windows-msvc-vulkan.zip" ;;
  esac
  if [ -n "$vulkan_runtime_name" ] && [ -f "$artifact_dir/$vulkan_runtime_name" ]; then
    vulkan_runtime_checksum="$(checksum_for_binary "$vulkan_runtime_name")"
  fi
  mac_pkg_name=""
  mac_pkg_checksum=""
  case "$binary_name" in
    opengpu-aarch64-apple-darwin) mac_pkg_name="MundusX-Node-aarch64-dev.pkg" ;;
  esac
  if [ -n "$mac_pkg_name" ] && [ -f "$artifact_dir/$mac_pkg_name" ]; then
    mac_pkg_checksum="$(checksum_for_binary "$mac_pkg_name")"
  fi
  generated_at="$("$(python_cmd)" - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"))
PY
)"

  "$(python_cmd)" - "$manifest_path" "$binary_name" "$checksum" "$tag_text" "$version_text" "$generated_at" "$agent_name" "$agent_checksum" "$agent_install_as" "$tray_name" "$tray_checksum" "$runtime_name" "$runtime_checksum" "$vulkan_runtime_name" "$vulkan_runtime_checksum" "$mac_pkg_name" "$mac_pkg_checksum" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
binary_name = sys.argv[2]
checksum = sys.argv[3]
tag = sys.argv[4]
version = sys.argv[5]
generated_at = sys.argv[6]
agent_name = sys.argv[7]
agent_checksum = sys.argv[8]
agent_install_as = sys.argv[9]
tray_name = sys.argv[10]
tray_checksum = sys.argv[11]
runtime_name = sys.argv[12]
runtime_checksum = sys.argv[13]
vulkan_runtime_name = sys.argv[14]
vulkan_runtime_checksum = sys.argv[15]
mac_pkg_name = sys.argv[16]
mac_pkg_checksum = sys.argv[17]

payload = {
    "artifact_kind": "release-binary",
    "binary_name": binary_name,
    "checksum_sha256": checksum,
    "generated_at": generated_at,
    "tag": tag,
    "version": version,
}
assets = []
if agent_name and agent_checksum:
    assets.append({
            "name": agent_name,
            "install_as": agent_install_as,
            "kind": "node-agent-binary",
            "checksum_sha256": agent_checksum,
        })
if tray_name and tray_checksum:
    assets.append({
        "name": tray_name,
        "install_as": "mundusx-tray.exe",
        "kind": "windows-tray-binary",
        "checksum_sha256": tray_checksum,
    })
if assets:
    payload["assets"] = assets
if mac_pkg_name and mac_pkg_checksum:
    payload.setdefault("installers", []).append({
        "name": mac_pkg_name,
        "kind": "macos-pkg-installer",
        "install_as": "MundusX Node Installer",
        "checksum_sha256": mac_pkg_checksum,
        "signed": False,
        "notarized": False,
    })
runtime_assets = []
if runtime_name and runtime_checksum:
    runtime_assets.append({
        "name": runtime_name,
        "install_as": "runtimes/llama",
        "kind": "llama-cpp-cuda-runtime-bundle",
        "checksum_sha256": runtime_checksum,
    })
if vulkan_runtime_name and vulkan_runtime_checksum:
    runtime_assets.append({
        "name": vulkan_runtime_name,
        "install_as": "runtimes/llama",
        "kind": "llama-cpp-vulkan-runtime-bundle",
        "checksum_sha256": vulkan_runtime_checksum,
    })
if runtime_assets:
    payload["runtime_assets"] = runtime_assets
manifest_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
PY
}

ensure_local_keys() {
  if [ -f "$private_key_path" ] && [ -f "$public_key_path" ]; then
    return 0
  fi

  if [ "${OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS:-0}" != "1" ]; then
    die "release signing keys are not configured; set OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS=1 for local preview or provide signing secrets in CI"
  fi

  mkdir -p "$(dirname "$private_key_path")"
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$private_key_path" >/dev/null 2>&1
  openssl pkey -in "$private_key_path" -pubout -out "$public_key_path" >/dev/null 2>&1
  generated_keys=1
}

decode_env_key() {
  local value="$1"
  local path="$2"
  local label="$3"
  "$(python_cmd)" - "$value" "$path" "$label" <<'PY'
import base64
import binascii
import sys
from pathlib import Path

value = sys.argv[1]
path = Path(sys.argv[2])
label = sys.argv[3]

try:
    key_bytes = base64.b64decode(value, validate=True)
except binascii.Error as exc:
    print(f"FAILED: invalid base64 release signing {label}: {exc}", file=sys.stderr)
    sys.exit(1)

path.write_bytes(key_bytes)
PY
}

load_private_key_for_sign() {
  if [ -n "${OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64:-}" ] && [ -n "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64:-}" ]; then
    local temp_dir
    temp_dir="$(mktemp -d)"
    private_key_path="$temp_dir/private.pem"
    public_key_path="$temp_dir/public.pem"
    decode_env_key "${OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64}" "$private_key_path" "private-key"
    decode_env_key "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64}" "$public_key_path" "public-key"
    return 0
  fi

  if [ -f "$private_key_path" ] && [ -f "$public_key_path" ]; then
    return 0
  fi

  ensure_local_keys
}

load_public_key_for_verify() {
  if [ -n "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64:-}" ]; then
    local temp_dir
    temp_dir="$(mktemp -d)"
    public_key_path="$temp_dir/public.pem"
    decode_env_key "${OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64}" "$public_key_path" "public-key"
    return 0
  fi

  if [ -f "$public_key_path" ]; then
    return 0
  fi

  if [ "${OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS:-0}" = "1" ]; then
    ensure_local_keys
    return 0
  fi

  die "release signing public key is not configured"
}

sign_manifest() {
  openssl dgst -sha256 -sign "$private_key_path" -out "$signature_path" "$manifest_path" >/dev/null 2>&1
}

verify_manifest() {
  [ -f "$manifest_path" ] || die "missing release manifest: $manifest_path"
  [ -f "$signature_path" ] || die "missing release manifest signature: $signature_path"
  openssl dgst -sha256 -verify "$public_key_path" -signature "$signature_path" "$manifest_path" >/dev/null 2>&1 \
    || die "release manifest signature verification failed"
}

case "$command" in
  prepare)
    [ -n "$artifact_dir" ] || die "artifact directory required"
    [ -n "$binary_name" ] || die "binary name required"
    create_manifest
    load_private_key_for_sign
    sign_manifest
    verify_manifest
    echo "Prepared signed release manifest:"
    echo "  manifest: $manifest_path"
    echo "  signature: $signature_path"
    if [ "$generated_keys" = "1" ]; then
      echo "  signing: local generated preview keys"
    fi
    ;;
  verify)
    [ -n "$artifact_dir" ] || die "artifact directory required"
    [ -n "$binary_name" ] || die "binary name required"
    [ -f "$manifest_path" ] || die "missing release manifest: $manifest_path"
    [ -f "$signature_path" ] || die "missing release manifest signature: $signature_path"
    load_public_key_for_verify
    verify_manifest
    echo "Verified signed release manifest:"
    echo "  manifest: $manifest_path"
    echo "  signature: $signature_path"
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
