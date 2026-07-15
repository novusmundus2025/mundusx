#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
install_dir="$tmp_dir/install"
mkdir -p "$bin_dir" "$install_dir"

cat >"$bin_dir/uname" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  -s) printf 'Darwin\n' ;;
  -m) printf 'arm64\n' ;;
  *) printf 'Darwin\n' ;;
esac
EOF
chmod +x "$bin_dir/uname"

cat >"$bin_dir/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

out=""
url=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      out="$2"
      shift 2
      ;;
    -*) shift ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

printf '%s\n' "$url" >>"$TEST_DOWNLOAD_LOG"

if [[ "$url" == *.sha256 ]]; then
  asset_name="$(basename "${url%.sha256}")"
  printf 'abc123  %s\n' "$asset_name" >"$out"
else
  printf 'placeholder\n' >"$out"
fi
EOF
chmod +x "$bin_dir/curl"

cat >"$bin_dir/shasum" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
exit 0
EOF
chmod +x "$bin_dir/shasum"

export PATH="$bin_dir:$PATH"
export TEST_DOWNLOAD_LOG="$tmp_dir/download.log"

output="$(
  INSTALL_DIR="$install_dir" \
  OPENGPU_SKIP_INSTALL_SMOKE=1 \
  bash "$repo_root/install.sh" 2>&1
)"

printf '%s\n' "$output" | grep -F "target: aarch64-apple-darwin" >/dev/null
grep -F "opengpu-aarch64-apple-darwin" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "opengpu-aarch64-apple-darwin.sha256" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "opengpu-node-agent-aarch64-apple-darwin" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "opengpu-node-agent-aarch64-apple-darwin.sha256" "$TEST_DOWNLOAD_LOG" >/dev/null
test -f "$install_dir/opengpu"
test -f "$install_dir/opengpu-node-agent"

echo "PASS: install.sh selects Apple Silicon macOS CLI and node-agent assets"
