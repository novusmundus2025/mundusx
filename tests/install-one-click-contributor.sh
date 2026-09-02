#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
asset_dir="$tmp_dir/assets"
install_dir="$tmp_dir/install"
mkdir -p "$bin_dir" "$asset_dir" "$install_dir"

cat >"$bin_dir/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) printf 'Linux\n' ;;
  -m) printf 'aarch64\n' ;;
  *) printf 'Linux\n' ;;
esac
EOF
chmod +x "$bin_dir/uname"

cat >"$asset_dir/opengpu-aarch64-unknown-linux-gnu" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$TEST_OPENGPU_LOG"
exit 0
EOF
cat >"$asset_dir/opengpu-node-agent-aarch64-unknown-linux-gnu" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$asset_dir"/opengpu-*

for asset in \
  opengpu-aarch64-unknown-linux-gnu \
  opengpu-node-agent-aarch64-unknown-linux-gnu
do
  (cd "$asset_dir" && sha256sum "$asset" >"$asset.sha256")
done

export TEST_OPENGPU_LOG="$tmp_dir/opengpu.log"
PATH="$bin_dir:$PATH" \
INSTALL_DIR="$install_dir" \
OPENGPU_GLOBAL_BIN_DIR="$bin_dir" \
bash "$repo_root/install.sh" \
  --local-assets "$asset_dir" \
  --without-vllm \
  --cap-percent 50 \
  --max-jobs 3 >/dev/null

grep -Fx -- '--version' "$TEST_OPENGPU_LOG" >/dev/null
grep -Fx -- 'install --public --cap-percent 50 --max-jobs 3 --no-contribute-cluster' "$TEST_OPENGPU_LOG" >/dev/null
grep -Fx -- 'onboarding --complete' "$TEST_OPENGPU_LOG" >/dev/null
grep -Fx -- 'start --background --max-jobs 3 --no-contribute-cluster' "$TEST_OPENGPU_LOG" >/dev/null
grep -Fx -- 'status' "$TEST_OPENGPU_LOG" >/dev/null

echo "PASS: install.sh configures and starts a public contributor with one command"
