#!/usr/bin/env bash
set -euo pipefail

artifact_dir="${1:-}"
binary_name="${2:-}"
agent_binary_name="${3:-}"
pkg_name="${4:-MundusX-OpenGPU-Apple-Silicon.pkg}"
version="${5:-0.1.0}"

usage() {
  cat <<'EOF'
Usage: scripts/package-macos-pkg.sh <artifact-dir> <opengpu-binary> <node-agent-binary> [pkg-name] [version]

Creates an unsigned macOS .pkg installer for Apple Silicon nodes. The package
installs opengpu and opengpu-node-agent to /usr/local/bin and links them into
the logged-in user's ~/.opengpu/bin directory during postinstall. On Apple
Silicon, postinstall also bootstraps python3 from python.org when it is missing,
so MLX setup can run during `opengpu install`.
EOF
}

die() {
  echo "FAILED: $*" >&2
  exit 1
}

if [ -z "$artifact_dir" ] || [ -z "$binary_name" ] || [ -z "$agent_binary_name" ]; then
  usage >&2
  exit 1
fi

command -v pkgbuild >/dev/null 2>&1 || die "pkgbuild is required; run this on macOS"

binary_path="$artifact_dir/$binary_name"
agent_binary_path="$artifact_dir/$agent_binary_name"
pkg_path="$artifact_dir/$pkg_name"

[ -f "$binary_path" ] || die "missing CLI binary: $binary_path"
[ -f "$agent_binary_path" ] || die "missing node-agent binary: $agent_binary_path"

work_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$work_dir"
}
trap cleanup EXIT

payload_dir="$work_dir/payload"
scripts_dir="$work_dir/scripts"
install_bin_dir="$payload_dir/usr/local/bin"
mkdir -p "$install_bin_dir" "$scripts_dir"

cp "$binary_path" "$install_bin_dir/opengpu"
cp "$agent_binary_path" "$install_bin_dir/opengpu-node-agent"
chmod 0755 "$install_bin_dir/opengpu" "$install_bin_dir/opengpu-node-agent"

cat > "$scripts_dir/postinstall" <<'POSTINSTALL'
#!/usr/bin/env bash
set -euo pipefail

console_user="$(stat -f %Su /dev/console 2>/dev/null || true)"
if [ -z "$console_user" ] || [ "$console_user" = "root" ]; then
  exit 0
fi

home_dir="$(dscl . -read "/Users/${console_user}" NFSHomeDirectory 2>/dev/null | awk '{print $2}' || true)"
if [ -z "$home_dir" ] || [ ! -d "$home_dir" ]; then
  exit 0
fi

opengpu_home="${home_dir}/.opengpu"
opengpu_bin="${opengpu_home}/bin"
mkdir -p "$opengpu_bin"
ln -sf /usr/local/bin/opengpu "${opengpu_bin}/opengpu"
ln -sf /usr/local/bin/opengpu-node-agent "${opengpu_bin}/opengpu-node-agent"
chown -R "${console_user}:staff" "$opengpu_home" 2>/dev/null || true

if [ "$(uname -s 2>/dev/null || true)" = "Darwin" ] && [ "$(uname -m 2>/dev/null || true)" = "arm64" ]; then
  if ! command -v python3 >/dev/null 2>&1 && [ "${OPENGPU_SKIP_PYTHON_BOOTSTRAP:-}" != "1" ]; then
    python_pkg_url="${OPENGPU_PYTHON_INSTALLER_URL:-https://www.python.org/ftp/python/3.12.4/python-3.12.4-macos11.pkg}"
    python_pkg="/tmp/opengpu-python3-macos.pkg"
    if /usr/bin/curl -fL --retry 3 -o "$python_pkg" "$python_pkg_url"; then
      /usr/sbin/installer -pkg "$python_pkg" -target / || true
      rm -f "$python_pkg"
    fi
  fi
fi

exit 0
POSTINSTALL
chmod 0755 "$scripts_dir/postinstall"

rm -f "$pkg_path"
pkgbuild \
  --root "$payload_dir" \
  --scripts "$scripts_dir" \
  --identifier "ai.mundusx.node" \
  --version "$version" \
  --install-location "/" \
  "$pkg_path"

pkgutil --expand "$pkg_path" "$work_dir/expanded" >/dev/null

echo "Created macOS package:"
echo "  pkg: $pkg_path"
