#!/usr/bin/env bash
set -euo pipefail

REPO="mundusx/mundusx"
BIN_NAME="opengpu"
COMPAT_BIN_NAME="mundusx"
DEFAULT_INSTALL_DIR="$HOME/.local/bin"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
GLOBAL_BIN_DIR_OVERRIDE="${OPENGPU_GLOBAL_BIN_DIR:-}"
GLOBAL_BIN_DIR="${OPENGPU_GLOBAL_BIN_DIR:-/usr/local/bin}"
RELEASE_BASE_URL="${RELEASE_BASE_URL:-https://github.com/${REPO}/releases/latest/download}"
OPENGPU_HOME="${OPENGPU_HOME:-$HOME/.opengpu}"
VLLM_IMAGE="${OPENGPU_VLLM_IMAGE:-nvcr.io/nvidia/vllm@sha256:63b808804826a028e38f559747a9e4d5985cf676616fbaa70c1937c58f83e13e}"
VLLM_IMAGE_TAG="${OPENGPU_VLLM_IMAGE_TAG:-26.06-py3}"
with_vllm=0
runtime_only=0
without_vllm=0
local_assets=""

usage() {
  cat <<'EOF'
Usage: install.sh [--with-vllm] [--without-vllm] [--runtime-only] [--local-assets DIR] [--help]

  --with-vllm    Install the pinned NVIDIA vLLM container runtime after the CLI.
  --without-vllm Skip automatic vLLM installation on detected GB10/GX10 hosts.
  --runtime-only Install only the vLLM runtime configuration (implies --with-vllm).
  --local-assets Install release binaries and checksums directly from DIR.
  --help         Show this help.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --with-vllm)
      with_vllm=1
      ;;
    --without-vllm)
      without_vllm=1
      ;;
    --runtime-only)
      with_vllm=1
      runtime_only=1
      ;;
    --local-assets)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        echo "--local-assets requires a directory" >&2
        usage >&2
        exit 1
      fi
      local_assets="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown installer option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os" in
  darwin) platform="apple-darwin" ;;
  linux) platform="unknown-linux-gnu" ;;
  mingw*|msys*|cygwin*) echo "Windows installs must use PowerShell: powershell -ExecutionPolicy Bypass -File .\\install.ps1" >&2; exit 1 ;;
  *) echo "unsupported operating system: $os" >&2; exit 1 ;;
esac

if [ "$without_vllm" -eq 0 ] \
  && [ "$with_vllm" -eq 0 ] \
  && [ "$os" = "linux" ] \
  && { [ "$arch" = "aarch64" ] || [ "$arch" = "arm64" ]; } \
  && command -v nvidia-smi >/dev/null 2>&1 \
  && nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | grep -Fq "GB10"; then
  with_vllm=1
  echo "Detected NVIDIA GB10/GX10; including the pinned vLLM runtime."
fi

case "$arch" in
  arm64|aarch64)
    target="aarch64-${platform}"
    ;;
  x86_64|amd64)
    if [ "$os" = "darwin" ]; then
      echo "current Mac release channel is Apple Silicon only; please use an M-series Mac or build from source" >&2
      exit 1
    fi
    target="x86_64-${platform}"
    ;;
  *)
    echo "unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

asset_name="${BIN_NAME}-${target}"
agent_asset_name="opengpu-node-agent-${target}"
if [ -n "$local_assets" ]; then
  if [ ! -d "$local_assets" ]; then
    echo "local asset directory not found: $local_assets" >&2
    exit 1
  fi
  release_source="${local_assets%/}"
else
  release_source="${RELEASE_BASE_URL%/}"
fi
release_url="${release_source}/${asset_name}"
checksum_url="${release_url}.sha256"
agent_url="${release_source}/${agent_asset_name}"
agent_checksum_url="${agent_url}.sha256"
tmp_dir="$(mktemp -d)"
tmp_bin="${tmp_dir}/${asset_name}"
tmp_checksum="${tmp_dir}/${asset_name}.sha256"
tmp_agent="${tmp_dir}/${agent_asset_name}"
tmp_agent_checksum="${tmp_dir}/${agent_asset_name}.sha256"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

if [ "$runtime_only" -eq 0 ]; then
  mkdir -p "$INSTALL_DIR"
fi

download_to() {
  local source="$1"
  local output="$2"

  if [ -n "$local_assets" ]; then
    if [ ! -f "$source" ]; then
      echo "local release asset not found: $source" >&2
      exit 1
    fi
    cp "$source" "$output"
    return
  fi

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$source" -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$output" "$source"
  else
    echo "curl or wget is required" >&2
    exit 1
  fi
}

verify_checksum() {
  local checksum_file="$1"

  if command -v shasum >/dev/null 2>&1; then
    (cd "$tmp_dir" && shasum -a 256 -c "$(basename "$checksum_file")")
  elif command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp_dir" && sha256sum -c "$(basename "$checksum_file")")
  else
    echo "checksum tooling not found (need shasum or sha256sum)" >&2
    exit 1
  fi
}

smoke_installed_binary() {
  local binary="$1"
  local label="$2"

  if [ "${OPENGPU_SKIP_INSTALL_SMOKE:-}" = "1" ]; then
    return 0
  fi

  if ! "$binary" --version >/dev/null 2>&1; then
    echo "installed ${label} failed to run: ${binary} --version" >&2
    echo "This usually means the downloaded release asset does not match this machine." >&2
    exit 1
  fi
}

path_contains_dir() {
  case ":${PATH}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

expose_installed_commands() {
  if [ "$INSTALL_DIR" != "$DEFAULT_INSTALL_DIR" ] && [ -z "$GLOBAL_BIN_DIR_OVERRIDE" ]; then
    return
  fi
  if path_contains_dir "$INSTALL_DIR"; then
    return
  fi
  if ! path_contains_dir "$GLOBAL_BIN_DIR"; then
    echo "Installed commands are not on PATH; add ${INSTALL_DIR} to PATH." >&2
    return
  fi

  echo "Making opengpu available immediately through ${GLOBAL_BIN_DIR}..."
  if [ -d "$GLOBAL_BIN_DIR" ] && [ -w "$GLOBAL_BIN_DIR" ]; then
    ln -sf "$INSTALL_DIR/$BIN_NAME" "$GLOBAL_BIN_DIR/$BIN_NAME"
    ln -sf "$INSTALL_DIR/$COMPAT_BIN_NAME" "$GLOBAL_BIN_DIR/$COMPAT_BIN_NAME"
    ln -sf "$INSTALL_DIR/opengpu-node-agent" "$GLOBAL_BIN_DIR/opengpu-node-agent"
  elif command -v sudo >/dev/null 2>&1; then
    sudo mkdir -p "$GLOBAL_BIN_DIR"
    sudo ln -sf "$INSTALL_DIR/$BIN_NAME" "$GLOBAL_BIN_DIR/$BIN_NAME"
    sudo ln -sf "$INSTALL_DIR/$COMPAT_BIN_NAME" "$GLOBAL_BIN_DIR/$COMPAT_BIN_NAME"
    sudo ln -sf "$INSTALL_DIR/opengpu-node-agent" "$GLOBAL_BIN_DIR/opengpu-node-agent"
  else
    echo "Cannot write ${GLOBAL_BIN_DIR}; rerun with ${INSTALL_DIR} on PATH." >&2
    return
  fi
  echo "Commands are available now; no terminal restart is required."
}

install_vllm_runtime() {
  local runtime_dir="${OPENGPU_HOME}/runtimes/vllm"
  local config_path="${runtime_dir}/runtime.conf"

  if [ "$os" != "linux" ]; then
    echo "vLLM runtime installation is supported only on Linux" >&2
    exit 1
  fi
  if [ "$arch" != "aarch64" ] && [ "$arch" != "arm64" ]; then
    echo "the first MundusX vLLM runtime target requires Linux ARM64 (GB10/GX10)" >&2
    exit 1
  fi
  if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required for the MundusX vLLM runtime" >&2
    exit 1
  fi
  if ! command -v nvidia-ctk >/dev/null 2>&1; then
    echo "NVIDIA Container Toolkit is required (nvidia-ctk was not found)" >&2
    exit 1
  fi
  if ! command -v nvidia-smi >/dev/null 2>&1 || ! nvidia-smi >/dev/null 2>&1; then
    echo "the NVIDIA driver is not ready: nvidia-smi could not access a GPU" >&2
    exit 1
  fi
  if ! docker info >/dev/null 2>&1; then
    echo "Docker is installed, but the current user cannot access the Docker daemon" >&2
    echo "On DGX Spark/GX10, add the user to the docker group or run the installer with an accessible daemon." >&2
    exit 1
  fi

  echo
  echo "Validating NVIDIA GPU access inside Docker..."
  docker run --rm --gpus all \
    nvcr.io/nvidia/cuda:13.0.1-base-ubuntu24.04 \
    nvidia-smi >/dev/null

  echo "Pulling pinned NVIDIA vLLM runtime (${VLLM_IMAGE_TAG})..."
  docker pull "$VLLM_IMAGE"

  mkdir -p "$runtime_dir" "${OPENGPU_HOME}/models"
  cat >"$config_path" <<EOF
# Managed by the MundusX installer.
VLLM_IMAGE=${VLLM_IMAGE}
VLLM_IMAGE_TAG=${VLLM_IMAGE_TAG}
VLLM_CONTAINER_NAME=mundusx-vllm
VLLM_BIND_ADDRESS=127.0.0.1
VLLM_PORT=8000
VLLM_GPU_MEMORY_UTILIZATION=0.70
VLLM_MAX_NUM_SEQS=4
VLLM_START_TIMEOUT_SECONDS=1800
VLLM_MODEL_DIR=${OPENGPU_HOME}/models
EOF
  chmod 600 "$config_path"

  echo "Installed vLLM runtime configuration to ${config_path}"
  echo "The node agent will own container startup and shutdown for connected vLLM nodes."
}

echo "MundusX installer"
echo "  target: ${target}"
echo "  source: ${release_source}"
echo "  node agent: ${agent_asset_name}"
echo "  install: ${INSTALL_DIR}"

if [ "$runtime_only" -eq 0 ]; then
  echo
  echo "Fetching ${BIN_NAME}..."
  download_to "$release_url" "$tmp_bin"

  download_to "$checksum_url" "$tmp_checksum"
  echo "Verifying checksum..."
  verify_checksum "$tmp_checksum"

  echo "Fetching opengpu-node-agent..."
  download_to "$agent_url" "$tmp_agent"
  download_to "$agent_checksum_url" "$tmp_agent_checksum"
  echo "Verifying node agent checksum..."
  verify_checksum "$tmp_agent_checksum"

  chmod +x "$tmp_bin"
  chmod +x "$tmp_agent"
  mv "$tmp_bin" "$INSTALL_DIR/$BIN_NAME"
  mv "$tmp_agent" "$INSTALL_DIR/opengpu-node-agent"
  ln -sf "$BIN_NAME" "$INSTALL_DIR/$COMPAT_BIN_NAME"
  expose_installed_commands

  echo "Running installed binary smoke checks..."
  smoke_installed_binary "$INSTALL_DIR/$BIN_NAME" "$BIN_NAME"
  smoke_installed_binary "$INSTALL_DIR/opengpu-node-agent" "opengpu-node-agent"

  echo
  echo "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
  echo "Installed ${COMPAT_BIN_NAME} compatibility alias to ${INSTALL_DIR}/${COMPAT_BIN_NAME}"
  echo "Installed opengpu-node-agent to ${INSTALL_DIR}/opengpu-node-agent"
fi

if [ "$with_vllm" -eq 1 ]; then
  install_vllm_runtime
fi

if [ "$runtime_only" -eq 0 ]; then
  echo
  echo "Next steps:"
  echo "  opengpu install"
  echo "  opengpu start"
fi
