#!/usr/bin/env bash
set -euo pipefail

BIN_NAME="opengpu"
COMPAT_BIN_NAME="mundusx"
DEFAULT_INSTALL_DIR="$HOME/.local/bin"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"
GLOBAL_BIN_DIR_OVERRIDE="${OPENGPU_GLOBAL_BIN_DIR:-}"
GLOBAL_BIN_DIR="${OPENGPU_GLOBAL_BIN_DIR:-/usr/local/bin}"
RELEASE_BASE_URL="${RELEASE_BASE_URL:-https://github.com/mundusx/releases/releases/download/opengpu-prod}"
OPENGPU_HOME="${OPENGPU_HOME:-$HOME/.opengpu}"
VLLM_IMAGE="${OPENGPU_VLLM_IMAGE:-nvcr.io/nvidia/vllm@sha256:63b808804826a028e38f559747a9e4d5985cf676616fbaa70c1937c58f83e13e}"
VLLM_IMAGE_TAG="${OPENGPU_VLLM_IMAGE_TAG:-26.06-py3}"
with_vllm=0
runtime_only=0
without_vllm=0
install_only=1
cap_percent="${OPENGPU_CAP_PERCENT:-30}"
max_jobs="${OPENGPU_MAX_JOBS:-2}"
local_assets=""
configure_service_only=0

# BEGIN MUNDUSX CHAT SERVICE
# Embedded in install.sh so curl-based installations need no extra downloads.
configure_chat_service() {
  [ "${MUNDUSX_SKIP_CHAT_SERVICE:-0}" != 1 ] || return 0
  local platform="${MUNDUSX_SERVICE_PLATFORM:-$(uname -s)}"
  local data="${MUNDUSX_HOME:-$HOME/.mundusx}"
  local bin="${INSTALL_DIR:-$HOME/.local/bin}"
  case "$data$bin$HOME" in *$'\n'*|*$'\r'*) echo 'Unsupported newline in installation path' >&2; return 1;; esac
  case "$platform" in Darwin|Linux) ;; *) echo 'Chat service requires macOS or Linux' >&2; return 1;; esac
  [ -x "$bin/mundusx" ] || { echo 'Install the MundusX connector first' >&2; return 1; }
  mkdir -p "$data/logs" "$data/bin"
  chmod 700 "$data"
  local runner="$data/bin/mundusx-chat-service" reconnect="$data/bin/mundusx-reconnect"
  # Keep credentials in the existing pairing file, never in service definitions.
  {
    printf '#!/usr/bin/env bash\nset -eu\nexport MUNDUSX_HOME=%q\n' "$data"
    printf 'export PATH=%q:$PATH\n' "$bin:/usr/local/bin:/opt/homebrew/bin"
    printf '[ -s "$MUNDUSX_HOME/chat-connection.json" ] || exit 0\n'
    printf 'exec %q connect\n' "$bin/mundusx"
  } > "$runner"
  {
    printf '#!/usr/bin/env bash\nset -eu\n'
    printf 'case "${1:-mundusx://reconnect}" in mundusx://reconnect|mundusx://reconnect/) ;; *) echo "Unsupported MundusX action" >&2; exit 2;; esac\n'
    if [ "$platform" = Linux ]; then
      printf 'exec systemctl --user start mundusx-chat.service\n'
    else
      printf 'launchctl bootstrap "gui/$(id -u)" %q 2>/dev/null || true\n' "$HOME/Library/LaunchAgents/ai.mundusx.chat.plist"
      # Deliberately omit -k: a live connector must not be killed.
      printf 'exec launchctl kickstart "gui/$(id -u)/ai.mundusx.chat"\n'
    fi
  } > "$reconnect"
  chmod 755 "$runner" "$reconnect"
  if [ "$platform" = Linux ]; then
    local config="${XDG_CONFIG_HOME:-$HOME/.config}" share="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$config/systemd/user" "$share/applications"
    local unit_path desktop_path
    unit_path=$(printf '%s' "$runner" | sed 's/\\/\\\\/g;s/"/\\"/g;s/%/%%/g')
    desktop_path=$(printf '%s' "$reconnect" | sed 's/\\/\\\\\\\\/g;s/"/\\"/g;s/`/\\`/g;s/\$/\\$/g;s/%/%%/g')
    cat > "$config/systemd/user/mundusx-chat.service" <<EOF
[Unit]
Description=MundusX saved Chat connection
StartLimitIntervalSec=0
[Service]
ExecStart=/bin/bash "$unit_path"
Restart=on-failure
RestartSec=5
TimeoutStopSec=20
[Install]
WantedBy=default.target
EOF
    cat > "$share/applications/mundusx-reconnect.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=MundusX Reconnect
Exec="$desktop_path" %u
Terminal=false
NoDisplay=true
MimeType=x-scheme-handler/mundusx;
EOF
    if [ "${MUNDUSX_CHAT_SERVICE_ACTIVATE:-1}" = 1 ]; then
    command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications" || true
    command -v xdg-mime >/dev/null && xdg-mime default mundusx-reconnect.desktop x-scheme-handler/mundusx || true
    if command -v systemctl >/dev/null && systemctl --user daemon-reload && systemctl --user enable --now mundusx-chat.service; then
      echo 'Configured Linux user-session Chat recovery and reconnect handler.'
    else
      echo 'Chat service files installed, but no working systemd user session was found. Run mundusx connect manually on this host.' >&2
    fi
    fi
  else
    local agents="$HOME/Library/LaunchAgents" app="$HOME/Applications/MundusX Reconnect.app"
    mkdir -p "$agents" "$HOME/Applications"
    local xml_runner xml_log
    xml_runner=$(printf '%s' "$runner" | sed 's/\&/\&amp;/g;s/</\&lt;/g;s/>/\&gt;/g')
    xml_log=$(printf '%s' "$data/logs/connector.log" | sed 's/\&/\&amp;/g;s/</\&lt;/g;s/>/\&gt;/g')
    cat > "$agents/ai.mundusx.chat.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>ai.mundusx.chat</string>
<key>ProgramArguments</key><array><string>/bin/bash</string><string>$xml_runner</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>5</integer>
<key>StandardOutPath</key><string>$xml_log</string>
<key>StandardErrorPath</key><string>$xml_log</string>
</dict></plist>
EOF
    local source_file escaped
    source_file=$(mktemp)
    escaped=$(printf '%s' "$reconnect" | sed 's/\\/\\\\/g;s/"/\\"/g')
    cat > "$source_file" <<EOF
on open location targetURL
  if targetURL is not "mundusx://reconnect" and targetURL is not "mundusx://reconnect/" then error "Unsupported MundusX action"
  do shell script quoted form of "$escaped" & " " & quoted form of targetURL
end open location
on run
  do shell script quoted form of "$escaped"
end run
EOF
    osacompile -o "$app" "$source_file"
    rm -f "$source_file"
    /usr/libexec/PlistBuddy -c 'Delete :CFBundleIdentifier' "$app/Contents/Info.plist" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c 'Delete :CFBundleURLTypes' "$app/Contents/Info.plist" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c 'Add :CFBundleIdentifier string ai.mundusx.reconnect' "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Add :CFBundleURLTypes array' "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Add :CFBundleURLTypes:0 dict' "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Add :CFBundleURLTypes:0:CFBundleURLSchemes array' "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c 'Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string mundusx' "$app/Contents/Info.plist"
    if [ "${MUNDUSX_CHAT_SERVICE_ACTIVATE:-1}" = 1 ]; then
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$app"
    "$reconnect" || echo 'Reconnect registration is ready; the LaunchAgent will start at the next GUI login.' >&2
    fi
    echo 'Configured macOS login Chat recovery and reconnect handler.'
  fi
  echo 'Existing pairing is reused. For a new installation, run mundusx connect once to approve this computer, then use Reconnect.'
}
# END MUNDUSX CHAT SERVICE

usage() {
  cat <<'EOF'
Usage: install.sh [--with-vllm] [--without-vllm] [--auto-start] [--install-only] [--cap-percent N] [--max-jobs N] [--runtime-only] [--local-assets DIR] [--help]

  --with-vllm    Install the pinned NVIDIA vLLM container runtime after the CLI.
  --without-vllm Skip automatic vLLM installation on detected GB10/GX10 hosts.
  --auto-start   Unattended mode: configure safe defaults and start the node.
  --install-only Install binaries/runtime only (default; retained for scripts).
  --cap-percent  Contribution cap used with --auto-start (default: 30).
  --max-jobs     Concurrent job limit used with --auto-start (default: 2).
  --runtime-only Install only the vLLM runtime configuration (implies --with-vllm).
  --local-assets Install release binaries and checksums directly from DIR.
  --configure-chat-service Configure per-user Chat startup/reconnect for existing binaries.
  --help         Show this help.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --configure-chat-service)
      configure_service_only=1
      ;;
    --with-vllm)
      with_vllm=1
      ;;
    --without-vllm)
      without_vllm=1
      ;;
    --install-only)
      install_only=1
      ;;
    --auto-start)
      install_only=0
      ;;
    --cap-percent)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        echo "--cap-percent requires a whole number from 1 through 80" >&2
        exit 1
      fi
      cap_percent="$2"
      shift
      ;;
    --max-jobs)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        echo "--max-jobs requires a positive whole number" >&2
        exit 1
      fi
      max_jobs="$2"
      shift
      ;;
    --runtime-only)
      with_vllm=1
      runtime_only=1
      ;;
    --local-assets)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        echo "--local-assets requires a directory" >&2
        usage >&2
        exit 1
      fi
      local_assets="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown installer option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

if [ "$configure_service_only" -eq 1 ]; then
  configure_chat_service
  exit 0
fi

case "$cap_percent" in
  ''|*[!0-9]*) echo "--cap-percent must be a whole number from 1 through 80" >&2; exit 1 ;;
esac
if [ "$cap_percent" -lt 1 ] || [ "$cap_percent" -gt 80 ]; then
  echo "--cap-percent must be a whole number from 1 through 80" >&2
  exit 1
fi
case "$max_jobs" in
  ''|*[!0-9]*|0) echo "--max-jobs must be a positive whole number" >&2; exit 1 ;;
esac

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os" in
  darwin) platform="apple-darwin" ;;
  linux) platform="unknown-linux-gnu" ;;
  mingw*|msys*|cygwin*) echo "Windows installs must use PowerShell: powershell -ExecutionPolicy Bypass -File .\\install.ps1" >&2; exit 1 ;;
  *) echo "unsupported operating system: $os" >&2; exit 1 ;;
esac

is_local_release_source() {
  [ -n "$local_assets" ] && return 0
  case "$RELEASE_BASE_URL" in
    http://127.0.0.1:*|http://localhost:*|file://*) return 0 ;;
    *) return 1 ;;
  esac
}

if [ "$without_vllm" -eq 0 ] \
  && [ "$with_vllm" -eq 0 ] \
  && [ "$os" = "linux" ] \
  && { [ "$arch" = "aarch64" ] || [ "$arch" = "arm64" ]; } \
  && command -v nvidia-smi >/dev/null 2>&1 \
  && nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | grep -Fq "GB10"; then
  with_vllm=1
  echo "Detected NVIDIA GB10/GX10; including the pinned vLLM runtime."
fi

case "$arch" in
  arm64|aarch64)
    target="aarch64-${platform}"
    ;;
  x86_64|amd64)
    if [ "$os" = "darwin" ] && ! is_local_release_source; then
      echo "current Mac release channel is Apple Silicon only; please use an M-series Mac or build from source" >&2
      exit 1
    fi
    target="x86_64-${platform}"
    ;;
  *)
    echo "unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

asset_name="${BIN_NAME}-${target}"
agent_asset_name="opengpu-node-agent-${target}"
mundusx_asset_name="mundusx-${target}"
agent_server_asset_name="mundusx-agent-server-${target}"
if [ -n "$local_assets" ]; then
  if [ ! -d "$local_assets" ]; then
    echo "local asset directory not found: $local_assets" >&2
    exit 1
  fi
  release_source="${local_assets%/}"
else
  release_source="${RELEASE_BASE_URL%/}"
fi
release_url="${release_source}/${asset_name}"
checksum_url="${release_url}.sha256"
agent_url="${release_source}/${agent_asset_name}"
agent_checksum_url="${agent_url}.sha256"
mundusx_url="${release_source}/${mundusx_asset_name}"
mundusx_checksum_url="${mundusx_url}.sha256"
agent_server_url="${release_source}/${agent_server_asset_name}"
agent_server_checksum_url="${agent_server_url}.sha256"
tmp_dir="$(mktemp -d)"
tmp_bin="${tmp_dir}/${asset_name}"
tmp_checksum="${tmp_dir}/${asset_name}.sha256"
tmp_agent="${tmp_dir}/${agent_asset_name}"
tmp_agent_checksum="${tmp_dir}/${agent_asset_name}.sha256"
tmp_mundusx="${tmp_dir}/${mundusx_asset_name}"
tmp_mundusx_checksum="${tmp_dir}/${mundusx_asset_name}.sha256"
tmp_agent_server="${tmp_dir}/${agent_server_asset_name}"
tmp_agent_server_checksum="${tmp_dir}/${agent_server_asset_name}.sha256"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

if [ "$runtime_only" -eq 0 ]; then
  mkdir -p "$INSTALL_DIR"
fi

download_to() {
  local source="$1"
  local output="$2"
  local label="$3"
  local started_at
  local finished_at
  local elapsed
  local bytes

  started_at="$(date +%s)"
  echo "Downloading ${label}..."

  if [ -n "$local_assets" ]; then
    if [ ! -f "$source" ]; then
      echo "local release asset not found: $source" >&2
      exit 1
    fi
    cp "$source" "$output"
  elif command -v curl >/dev/null 2>&1; then
    curl \
      --fail \
      --location \
      --retry 3 \
      --progress-bar \
      --show-error \
      "$source" \
      -o "$output"
  elif command -v wget >/dev/null 2>&1; then
    wget --progress=bar:force:noscroll -O "$output" "$source"
  else
    echo "curl or wget is required" >&2
    exit 1
  fi

  finished_at="$(date +%s)"
  elapsed=$((finished_at - started_at))
  bytes="$(wc -c <"$output" | tr -d '[:space:]')"
  echo "Downloaded ${label}: ${bytes} bytes in ${elapsed}s."
}

verify_checksum() {
  local checksum_file="$1"

  if command -v shasum >/dev/null 2>&1; then
    (cd "$tmp_dir" && shasum -a 256 -c "$(basename "$checksum_file")")
  elif command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp_dir" && sha256sum -c "$(basename "$checksum_file")")
  else
    echo "checksum tooling not found (need shasum or sha256sum)" >&2
    exit 1
  fi
}

smoke_installed_binary() {
  local binary="$1"
  local label="$2"

  if [ "${OPENGPU_SKIP_INSTALL_SMOKE:-}" = "1" ]; then
    return 0
  fi

  if ! "$binary" --version >/dev/null 2>&1; then
    echo "installed ${label} failed to run: ${binary} --version" >&2
    echo "This usually means the downloaded release asset does not match this machine." >&2
    exit 1
  fi
}

path_contains_dir() {
  case ":${PATH}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

expose_installed_commands() {
  if [ "$INSTALL_DIR" != "$DEFAULT_INSTALL_DIR" ] && [ -z "$GLOBAL_BIN_DIR_OVERRIDE" ]; then
    return
  fi
  if path_contains_dir "$INSTALL_DIR"; then
    return
  fi
  if ! path_contains_dir "$GLOBAL_BIN_DIR"; then
    echo "Installed commands are not on PATH; add ${INSTALL_DIR} to PATH." >&2
    return
  fi

  echo "Making opengpu available immediately through ${GLOBAL_BIN_DIR}..."
  if [ -d "$GLOBAL_BIN_DIR" ] && [ -w "$GLOBAL_BIN_DIR" ]; then
    ln -sf "$INSTALL_DIR/$BIN_NAME" "$GLOBAL_BIN_DIR/$BIN_NAME"
    ln -sf "$INSTALL_DIR/mundusx" "$GLOBAL_BIN_DIR/mundusx"
    ln -sf "$INSTALL_DIR/mundusx-agent-server" "$GLOBAL_BIN_DIR/mundusx-agent-server"
    ln -sf "$INSTALL_DIR/opengpu-node-agent" "$GLOBAL_BIN_DIR/opengpu-node-agent"
  elif command -v sudo >/dev/null 2>&1; then
    sudo mkdir -p "$GLOBAL_BIN_DIR"
    sudo ln -sf "$INSTALL_DIR/$BIN_NAME" "$GLOBAL_BIN_DIR/$BIN_NAME"
    sudo ln -sf "$INSTALL_DIR/mundusx" "$GLOBAL_BIN_DIR/mundusx"
    sudo ln -sf "$INSTALL_DIR/mundusx-agent-server" "$GLOBAL_BIN_DIR/mundusx-agent-server"
    sudo ln -sf "$INSTALL_DIR/opengpu-node-agent" "$GLOBAL_BIN_DIR/opengpu-node-agent"
  else
    echo "Cannot write ${GLOBAL_BIN_DIR}; rerun with ${INSTALL_DIR} on PATH." >&2
    return
  fi
  echo "Commands are available now; no terminal restart is required."
}

install_vllm_runtime() {
  local runtime_dir="${OPENGPU_HOME}/runtimes/vllm"
  local config_path="${runtime_dir}/runtime.conf"

  if [ "$os" != "linux" ]; then
    echo "vLLM runtime installation is supported only on Linux" >&2
    exit 1
  fi
  if [ "$arch" != "aarch64" ] && [ "$arch" != "arm64" ]; then
    echo "the first MundusX vLLM runtime target requires Linux ARM64 (GB10/GX10)" >&2
    exit 1
  fi
  if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required for the MundusX vLLM runtime" >&2
    exit 1
  fi
  if ! command -v nvidia-ctk >/dev/null 2>&1; then
    echo "NVIDIA Container Toolkit is required (nvidia-ctk was not found)" >&2
    exit 1
  fi
  if ! command -v nvidia-smi >/dev/null 2>&1 || ! nvidia-smi >/dev/null 2>&1; then
    echo "the NVIDIA driver is not ready: nvidia-smi could not access a GPU" >&2
    exit 1
  fi
  if ! docker info >/dev/null 2>&1; then
    echo "Docker is installed, but the current user cannot access the Docker daemon" >&2
    echo "On DGX Spark/GX10, add the user to the docker group or run the installer with an accessible daemon." >&2
    exit 1
  fi

  echo
  echo "[runtime 1/2] Validating NVIDIA GPU access inside Docker..."
  echo "Docker shows image-layer download progress if the CUDA image is not cached."
  docker run --rm --gpus all \
    nvcr.io/nvidia/cuda:13.0.1-base-ubuntu24.04 \
    nvidia-smi >/dev/null

  echo "[runtime 2/2] Pulling pinned NVIDIA vLLM runtime (${VLLM_IMAGE_TAG})..."
  echo "Docker reports every layer and shows what remains before completion."
  docker pull "$VLLM_IMAGE"

  mkdir -p "$runtime_dir" "${OPENGPU_HOME}/models"
  cat >"$config_path" <<EOF
# Managed by the MundusX installer.
VLLM_IMAGE=${VLLM_IMAGE}
VLLM_IMAGE_TAG=${VLLM_IMAGE_TAG}
VLLM_CONTAINER_NAME=mundusx-vllm
VLLM_BIND_ADDRESS=127.0.0.1
VLLM_PORT=8000
VLLM_GPU_MEMORY_UTILIZATION=0.70
VLLM_MAX_NUM_SEQS=4
VLLM_START_TIMEOUT_SECONDS=1800
VLLM_MODEL_DIR=${OPENGPU_HOME}/models
EOF
  chmod 600 "$config_path"

  echo "Installed vLLM runtime configuration to ${config_path}"
  echo "The node agent will own container startup and shutdown for connected vLLM nodes."
}

echo "MundusX installer"
echo "  target: ${target}"
echo "  source: ${release_source}"
echo "  node agent: ${agent_asset_name}"
echo "  install: ${INSTALL_DIR}"

if [ "$runtime_only" -eq 0 ]; then
  echo
  echo "[binary 1/2] OpenGPU CLI"
  download_to "$release_url" "$tmp_bin" "OpenGPU CLI"

  download_to "$checksum_url" "$tmp_checksum" "OpenGPU CLI checksum"
  echo "Verifying checksum..."
  verify_checksum "$tmp_checksum"

  echo "[binary 2/2] OpenGPU node agent"
  download_to "$agent_url" "$tmp_agent" "OpenGPU node agent"
  download_to "$agent_checksum_url" "$tmp_agent_checksum" "OpenGPU node-agent checksum"
  echo "Verifying node agent checksum..."
  verify_checksum "$tmp_agent_checksum"

  echo "Fetching MundusX agent..."
  download_to "$mundusx_url" "$tmp_mundusx" "MundusX agent"
  download_to "$mundusx_checksum_url" "$tmp_mundusx_checksum" "MundusX agent checksum"
  verify_checksum "$tmp_mundusx_checksum"
  download_to "$agent_server_url" "$tmp_agent_server" "MundusX agent server"
  download_to "$agent_server_checksum_url" "$tmp_agent_server_checksum" "MundusX agent server checksum"
  verify_checksum "$tmp_agent_server_checksum"

  chmod +x "$tmp_bin"
  chmod +x "$tmp_agent"
  chmod +x "$tmp_mundusx"
  chmod +x "$tmp_agent_server"
  mv "$tmp_bin" "$INSTALL_DIR/$BIN_NAME"
  mv "$tmp_agent" "$INSTALL_DIR/opengpu-node-agent"
  mv "$tmp_mundusx" "$INSTALL_DIR/mundusx"
  mv "$tmp_agent_server" "$INSTALL_DIR/mundusx-agent-server"
  expose_installed_commands
  configure_chat_service

  echo "Running installed binary smoke checks..."
  smoke_installed_binary "$INSTALL_DIR/$BIN_NAME" "$BIN_NAME"
  smoke_installed_binary "$INSTALL_DIR/opengpu-node-agent" "opengpu-node-agent"
  smoke_installed_binary "$INSTALL_DIR/mundusx" "mundusx"
  smoke_installed_binary "$INSTALL_DIR/mundusx-agent-server" "mundusx-agent-server"

  echo
  echo "Installed ${BIN_NAME} to ${INSTALL_DIR}/${BIN_NAME}"
  echo "Installed MundusX agent to ${INSTALL_DIR}/mundusx"
  echo "Installed MundusX agent server to ${INSTALL_DIR}/mundusx-agent-server"
  echo "Installed opengpu-node-agent to ${INSTALL_DIR}/opengpu-node-agent"
fi

if [ "$with_vllm" -eq 1 ]; then
  install_vllm_runtime
fi

if [ "$runtime_only" -eq 0 ] && [ "$install_only" -eq 0 ]; then
  echo
  echo "Configuring this machine as a public MundusX contributor..."
  "$INSTALL_DIR/$BIN_NAME" install \
    --public \
    --cap-percent "$cap_percent" \
    --max-jobs "$max_jobs" \
    --no-contribute-cluster </dev/null
  "$INSTALL_DIR/$BIN_NAME" onboarding --complete

  echo
  echo "Starting the OpenGPU node in the background..."
  "$INSTALL_DIR/$BIN_NAME" start \
    --background \
    --max-jobs "$max_jobs" \
    --no-contribute-cluster </dev/null

  echo
  echo "OpenGPU is installed and contributing."
  "$INSTALL_DIR/$BIN_NAME" status
elif [ "$runtime_only" -eq 0 ]; then
  echo
  echo "Next steps:"
  echo "  opengpu install"
  echo "  opengpu start"
fi
