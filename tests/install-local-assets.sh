#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
asset_dir="$tmp_dir/local assets"
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

for asset in \
  opengpu-aarch64-unknown-linux-gnu \
  opengpu-node-agent-aarch64-unknown-linux-gnu
do
  printf '#!/usr/bin/env bash\nexit 0\n' >"$asset_dir/$asset"
  chmod +x "$asset_dir/$asset"
  (
    cd "$asset_dir"
    sha256sum "$asset" >"$asset.sha256"
  )
done

output="$(
  PATH="$bin_dir:$PATH" \
  INSTALL_DIR="$install_dir" \
  OPENGPU_GLOBAL_BIN_DIR="$bin_dir" \
  OPENGPU_SKIP_INSTALL_SMOKE=1 \
  bash "$repo_root/install.sh" --local-assets "$asset_dir" --without-vllm --install-only 2>&1
)"

printf '%s\n' "$output" | grep -F "source: $asset_dir" >/dev/null
printf '%s\n' "$output" | grep -F "no terminal restart is required" >/dev/null
test -x "$install_dir/opengpu"
test -x "$install_dir/opengpu-node-agent"
test -L "$install_dir/mundusx"
test -L "$bin_dir/opengpu"
test -L "$bin_dir/mundusx"
test -L "$bin_dir/opengpu-node-agent"

echo "PASS: install.sh installs verified binaries directly from a local asset directory"
