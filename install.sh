#!/usr/bin/env bash
set -euo pipefail

REPO="novusmundus2025/opengpu"
BIN_NAME="opengpu"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os" in
  darwin) platform="apple-darwin" ;;
  linux) platform="unknown-linux-gnu" ;;
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
release_url="https://github.com/${REPO}/releases/latest/download/${asset_name}"
checksum_url="${release_url}.sha256"
tmp_dir="$(mktemp -d)"
tmp_bin="${tmp_dir}/${asset_name}"
tmp_checksum="${tmp_dir}/${asset_name}.sha256"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

mkdir -p "$INSTALL_DIR"

echo "Downloading ${BIN_NAME} for ${target}..."
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$release_url" -o "$tmp_bin"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp_bin" "$release_url"
else
  echo "curl or wget is required" >&2
  exit 1
fi

if command -v curl >/dev/null 2>&1; then
  if curl -fsSL "$checksum_url" -o "$tmp_checksum"; then
    if command -v shasum >/dev/null 2>&1; then
      (cd "$tmp_dir" && shasum -a 256 -c "$(basename "$tmp_checksum")")
    elif command -v sha256sum >/dev/null 2>&1; then
      (cd "$tmp_dir" && sha256sum -c "$(basename "$tmp_checksum")")
    else
      echo "checksum verification skipped: no shasum or sha256sum available" >&2
    fi
  else
    echo "checksum unavailable for ${asset_name}, continuing without verification" >&2
  fi
fi

chmod +x "$tmp_bin"
mv "$tmp_bin" "$INSTALL_DIR/$BIN_NAME"

echo "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
echo "Make sure ${INSTALL_DIR} is on your PATH."
