#!/usr/bin/env bash
set -euo pipefail

REPO="mundusx/mundusx"
BIN_NAME="opengpu"
COMPAT_BIN_NAME="mundusx"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
RELEASE_BASE_URL="${RELEASE_BASE_URL:-https://github.com/${REPO}/releases/latest/download}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os" in
  darwin) platform="apple-darwin" ;;
  linux) platform="unknown-linux-gnu" ;;
  mingw*|msys*|cygwin*) echo "Windows installs must use PowerShell: powershell -ExecutionPolicy Bypass -File .\\install.ps1" >&2; exit 1 ;;
  *) echo "unsupported operating system: $os" >&2; exit 1 ;;
esac

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
release_url="${RELEASE_BASE_URL%/}/${asset_name}"
checksum_url="${release_url}.sha256"
agent_url="${RELEASE_BASE_URL%/}/${agent_asset_name}"
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

mkdir -p "$INSTALL_DIR"

download_to() {
  local url="$1"
  local output="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$output" "$url"
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

echo "MundusX installer"
echo "  target: ${target}"
echo "  source: ${RELEASE_BASE_URL%/}"
echo "  node agent: ${agent_asset_name}"
echo "  install: ${INSTALL_DIR}"
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

echo "Running installed binary smoke checks..."
smoke_installed_binary "$INSTALL_DIR/$BIN_NAME" "$BIN_NAME"
smoke_installed_binary "$INSTALL_DIR/opengpu-node-agent" "opengpu-node-agent"

echo
echo "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
echo "Installed ${COMPAT_BIN_NAME} compatibility alias to ${INSTALL_DIR}/${COMPAT_BIN_NAME}"
echo "Installed opengpu-node-agent to ${INSTALL_DIR}/opengpu-node-agent"
echo "If needed, add ${INSTALL_DIR} to your PATH."
