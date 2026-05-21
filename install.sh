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
  arm64|aarch64) target="aarch64-${platform}" ;;
  x86_64|amd64) target="x86_64-${platform}" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac

release_url="https://github.com/${REPO}/releases/latest/download/${BIN_NAME}-${target}"
tmp_dir="$(mktemp -d)"
tmp_bin="${tmp_dir}/${BIN_NAME}"

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

chmod +x "$tmp_bin"
mv "$tmp_bin" "$INSTALL_DIR/$BIN_NAME"

echo "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
echo "Make sure ${INSTALL_DIR} is on your PATH."
