#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
install_dir="$tmp_dir/install"
mkdir -p "$bin_dir" "$install_dir"

for tool in bash uname tr mktemp rm mkdir chmod mv basename grep; do
  ln -s "$(command -v "$tool")" "$bin_dir/$tool"
done

cat >"$bin_dir/wget" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

out=""
url=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    -qO)
      out="$2"
      url="$3"
      shift 3
      ;;
    *)
      shift
      ;;
  esac
done

printf '%s\n' "$url" >>"$TEST_WGET_LOG"

if [[ "$url" == *.sha256 ]]; then
  asset_name="$(basename "${url%.sha256}")"
  printf 'abc123  %s\n' "$asset_name" >"$out"
else
  printf 'placeholder\n' >"$out"
fi
EOF
chmod +x "$bin_dir/wget"

cat >"$bin_dir/shasum" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$TEST_SHASUM_LOG"
exit 0
EOF
chmod +x "$bin_dir/shasum"

export PATH="$bin_dir"
export TEST_WGET_LOG="$tmp_dir/wget.log"
export TEST_SHASUM_LOG="$tmp_dir/shasum.log"

output="$(
  INSTALL_DIR="$install_dir" \
  bash "$repo_root/install.sh" 2>&1
)"

expected_asset="https://github.com/mundusx/mundusx/releases/latest/download/opengpu-"
expected_checksum="https://github.com/mundusx/mundusx/releases/latest/download/opengpu-"
grep -F "$expected_asset" "$TEST_WGET_LOG" >/dev/null
grep -F "$expected_checksum" "$TEST_WGET_LOG" | grep -F ".sha256" >/dev/null
test -s "$TEST_SHASUM_LOG"
test -x "$install_dir/opengpu"
printf '%s\n' "$output" | grep -F "Verifying checksum..." >/dev/null

echo "PASS: install.sh verifies checksums when wget is the available downloader"
