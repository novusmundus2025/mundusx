#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
install_dir="$tmp_dir/install"
mkdir -p "$bin_dir" "$install_dir"

original_path="$PATH"

cat >"$bin_dir/uname" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
  -s) printf 'Linux\n' ;;
  -m) printf 'x86_64\n' ;;
  *) printf 'Linux\n' ;;
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
printf '%s\n' "$*" >>"$TEST_SHASUM_LOG"
exit 0
EOF
chmod +x "$bin_dir/shasum"

export PATH="$bin_dir:$original_path"
export TEST_DOWNLOAD_LOG="$tmp_dir/download.log"
export TEST_SHASUM_LOG="$tmp_dir/shasum.log"

output="$(
  INSTALL_DIR="$install_dir" \
  OPENGPU_SKIP_INSTALL_SMOKE=1 \
  bash "$repo_root/install.sh" --install-only 2>&1
)"

expected_asset="https://github.com/mundusx/releases/releases/download/opengpu-prod/opengpu-"
expected_checksum="https://github.com/mundusx/releases/releases/download/opengpu-prod/opengpu-"
expected_agent="https://github.com/mundusx/releases/releases/download/opengpu-prod/opengpu-node-agent-"
grep -F "$expected_asset" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "$expected_checksum" "$TEST_DOWNLOAD_LOG" | grep -F ".sha256" >/dev/null
grep -F "$expected_agent" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "$expected_agent" "$TEST_DOWNLOAD_LOG" | grep -F ".sha256" >/dev/null
test -s "$TEST_SHASUM_LOG"
test -f "$install_dir/opengpu"
test -f "$install_dir/opengpu-node-agent"
printf '%s\n' "$output" | grep -F "Verifying checksum..." >/dev/null
printf '%s\n' "$output" | grep -F "Verifying node agent checksum..." >/dev/null

echo "PASS: install.sh verifies CLI and node-agent checksums when wget is the available downloader"
