#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
install_dir="$tmp_dir/install"
mkdir -p "$bin_dir" "$install_dir"

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
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done

printf '%s\n' "$url" >>"$TEST_CURL_LOG"
printf 'placeholder\n' >"$out"
EOF
chmod +x "$bin_dir/curl"

cat >"$bin_dir/shasum" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
exit 0
EOF
chmod +x "$bin_dir/shasum"

export PATH="$bin_dir:$PATH"
export TEST_CURL_LOG="$tmp_dir/curl.log"

output="$(
  INSTALL_DIR="$install_dir" \
  OPENGPU_SKIP_INSTALL_SMOKE=1 \
  bash "$repo_root/install.sh" 2>&1
)"

expected_source="source: https://github.com/mundusx/releases/releases/download/opengpu-prod"
expected_asset="https://github.com/mundusx/releases/releases/download/opengpu-prod/opengpu-"

printf '%s\n' "$output" | grep -F "$expected_source" >/dev/null
grep -F "$expected_asset" "$TEST_CURL_LOG" >/dev/null

echo "PASS: install.sh defaults to the public OpenGPU production channel"
