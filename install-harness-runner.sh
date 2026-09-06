#!/usr/bin/env sh
set -eu

chat_url="${MUNDUSX_CHAT_URL:-https://chat.mundusx.ai}"
release_base="${MUNDUSX_HARNESS_RELEASE_URL:-https://downloads.mundusx.ai/prod/latest}"
install_dir="${MUNDUSX_HARNESS_INSTALL_DIR:-$HOME/.local/bin}"
command -v git >/dev/null 2>&1 || { echo "Git is required for local project isolation." >&2; exit 2; }
command -v curl >/dev/null 2>&1 || { echo "curl is required to download the Harness runner." >&2; exit 2; }
os="$(uname -s)"
arch="$(uname -m)"

case "$os:$arch" in
  Linux:x86_64) asset="mundusx-harness-runner-x86_64-unknown-linux-gnu" ;;
  Darwin:arm64) asset="mundusx-harness-runner-aarch64-apple-darwin" ;;
  *) echo "Unsupported Harness runner platform: $os $arch" >&2; exit 2 ;;
esac

temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT INT TERM
mkdir -p "$install_dir"

download() {
  curl --fail --location --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 120 \
    "$release_base/$1" -o "$temporary/$1"
}

download "$asset"
download "$asset.sha256"
if [ "$os" = "Linux" ]; then
  (cd "$temporary" && sha256sum -c "$asset.sha256")
else
  (cd "$temporary" && shasum -a 256 -c "$asset.sha256")
fi
install -m 0755 "$temporary/$asset" "$install_dir/mundusx-harness-runner"
"$install_dir/mundusx-harness-runner" bootstrap --chat-url "$chat_url"
