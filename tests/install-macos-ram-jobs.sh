#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
mkdir -p "$tmp_dir/bin" "$tmp_dir/assets" "$tmp_dir/install"
export TEST_OPENGPU_LOG="$tmp_dir/commands.log"
export PATH="$tmp_dir/bin:$PATH"
export INSTALL_DIR="$tmp_dir/install"
export MUNDUSX_SKIP_CHAT_SERVICE=1
unset OPENGPU_GLOBAL_BIN_DIR OPENGPU_MAX_JOBS

cat >"$tmp_dir/bin/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  -s) echo "${TEST_OS:-Darwin}" ;;
  -m) echo aarch64 ;;
esac
EOF
cat >"$tmp_dir/bin/sysctl" <<'EOF'
#!/usr/bin/env bash
[ "$*" = '-n hw.memsize' ] || exit 1
[ "$TEST_RAM" != failed ] || exit 1
echo "$TEST_RAM"
EOF
chmod +x "$tmp_dir/bin/"*

for platform in apple-darwin unknown-linux-gnu; do
  for binary in opengpu opengpu-node-agent mundusx mundusx-agent-server; do
    asset="$binary-aarch64-$platform"
    cat >"$tmp_dir/assets/$asset" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$TEST_OPENGPU_LOG"
EOF
    chmod +x "$tmp_dir/assets/$asset"
    (cd "$tmp_dir/assets" && sha256sum "$asset" >"$asset.sha256")
  done
done

check_jobs() {
  export TEST_RAM="$1"
  local expected="$2"
  shift 2
  : >"$TEST_OPENGPU_LOG"
  bash "${INSTALLER_UNDER_TEST:-$repo_root/install.sh}" --local-assets "$tmp_dir/assets" \
    --without-vllm --auto-start "$@" >"$tmp_dir/output.log" 2>&1 || {
      cat "$tmp_dir/output.log"
      return 1
    }
  grep -Fx -- "install --public --cap-percent 30 --max-jobs $expected --no-contribute-cluster" "$TEST_OPENGPU_LOG"
  grep -Fx -- "start --background --max-jobs $expected --no-contribute-cluster" "$TEST_OPENGPU_LOG"
}

for gib in 8 16 24 31 32 48 64 96 127 128 192; do
  expected=1
  if [ "$gib" -ge 128 ]; then expected=4
  elif [ "$gib" -ge 32 ]; then expected=2
  fi
  check_jobs "$((gib * 1073741824))" "$expected"
done
check_jobs failed 1
check_jobs invalid 1
check_jobs 8589934592 1 --max-jobs 4
check_jobs 68719476736 2 --max-jobs 4
check_jobs 137438953472 4 --max-jobs 8
check_jobs 137438953472 1 --max-jobs 1
export OPENGPU_MAX_JOBS=8
check_jobs 8589934592 1
check_jobs 137438953472 4
unset OPENGPU_MAX_JOBS
export TEST_OS=Linux
check_jobs failed 2
check_jobs failed 3 --max-jobs 3
echo 'PASS: Mac RAM caps setup and startup; Linux defaults and overrides are unchanged'
