#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/assets" "$tmp/tools" "$tmp/install"
printf '#!/usr/bin/env bash\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; esac\n' > "$tmp/tools/uname"
chmod +x "$tmp/tools/uname"
for name in opengpu opengpu-node-agent; do
  asset="$name-x86_64-unknown-linux-gnu"
  printf '#!/usr/bin/env bash\n[ "$1" = --version ]\n' > "$tmp/assets/$asset"
  chmod +x "$tmp/assets/$asset"
  (cd "$tmp/assets" && sha256sum "$asset" > "$asset.sha256")
done
export PATH="$tmp/tools:$PATH" INSTALL_DIR="$tmp/install" OPENGPU_HOME="$tmp/home"
bash "$root/install.sh" --without-vllm --local-assets "$tmp/assets" > "$tmp/log"
[ -x "$tmp/install/opengpu" ] && [ -x "$tmp/install/opengpu-node-agent" ]
[ ! -e "$tmp/install/mundusx" ] && [ ! -e "$tmp/install/mundusx-agent-server" ]
if grep -Eq 'mundusx connect|Chat recovery|Fetching MundusX agent' "$tmp/log"; then exit 1; fi
for name in mundusx mundusx-agent-server; do
  asset="$name-x86_64-unknown-linux-gnu"
  flag=--version
  [ "$name" != mundusx-agent-server ] || flag=--help
  printf '#!/usr/bin/env bash\n[ "$1" = %s ]\n' "$flag" > "$tmp/assets/$asset"
  chmod +x "$tmp/assets/$asset"
  (cd "$tmp/assets" && sha256sum "$asset" > "$asset.sha256")
done
MUNDUSX_SKIP_CHAT_SERVICE=1 bash "$root/install.sh" --without-vllm --with-chat-connector --local-assets "$tmp/assets" > "$tmp/optional-log"
[ -x "$tmp/install/mundusx-agent-server" ]
echo 'Contributor-only install and optional server --help check passed.'
