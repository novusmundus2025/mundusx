#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_name="opengpu"
bind_address="${LOCAL_RELEASE_BIND_ADDRESS:-127.0.0.1}"
port="${LOCAL_RELEASE_PORT:-8788}"
preview_root="${OPENGPU_LOCAL_RELEASE_PREVIEW_DIR:-$repo_root/.opengpu/local-release-preview}"
asset_dir="$preview_root/releases/latest/download"
server_pid_file="$preview_root/server.pid"
server_log_file="$preview_root/server.log"

usage() {
  cat <<'EOF'
Usage: scripts/local-release-preview.sh <build|verify|serve|up|stop|status|clean|url>

Commands:
  build   Build the release binary and materialize the local release tree.
  verify  Validate the release tree contents and checksum.
  serve   Build, then start the local HTTP preview in the background.
  up      Alias for serve.
  stop    Stop the background preview server if it is running.
  status  Report whether the local preview server is running.
  clean   Stop the server and remove the preview directory.
  url     Print the local release preview URL.
EOF
}

die() {
  echo "FAILED: $*" >&2
  exit 1
}

target_triplet() {
  local os arch platform
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    darwin) platform="apple-darwin" ;;
    linux) platform="unknown-linux-gnu" ;;
    *) die "unsupported operating system: $os" ;;
  esac

  case "$arch" in
    arm64|aarch64) printf 'aarch64-%s\n' "$platform" ;;
    x86_64|amd64)
      if [ "$os" = "darwin" ]; then
        die "current Mac release channel is Apple Silicon only; please use an M-series Mac or build from source"
      fi
      printf 'x86_64-%s\n' "$platform"
      ;;
    *) die "unsupported architecture: $arch" ;;
  esac
}

checksum_for() {
  local file="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  else
    die "checksum tooling not found (need shasum or sha256sum)"
  fi
}

preview_url() {
  printf 'http://%s:%s/releases/latest/download\n' "$bind_address" "$port"
}

build_preview() {
  local target asset_name binary_source binary_path checksum_path index_path checksum
  target="$(target_triplet)"
  asset_name="${bin_name}-${target}"
  binary_source="$repo_root/target/release/$bin_name"
  binary_path="$asset_dir/$asset_name"
  checksum_path="$binary_path.sha256"
  index_path="$asset_dir/index.html"

  mkdir -p "$asset_dir"

  echo "Building $bin_name release binary..."
  cargo build --release --manifest-path "$repo_root/apps/cli/Cargo.toml" >/dev/null

  [ -f "$binary_source" ] || die "missing built binary: $binary_source"
  cp "$binary_source" "$binary_path"
  chmod +x "$binary_path"

  checksum="$(checksum_for "$binary_path")"
  printf '%s  %s\n' "$checksum" "$(basename "$binary_path")" > "$checksum_path"
  OPENGPU_RELEASE_SIGNING_ALLOW_GENERATED_KEYS=1 \
    "$repo_root/scripts/release-signing.sh" prepare "$asset_dir" "$asset_name" "local-preview" "0.1.0"

  cat > "$index_path" <<EOF
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>OpenGPU Local Release Preview</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --line-strong: rgba(15, 23, 42, 0.14);
        --text: #0f172a;
        --muted: #5f6b85;
        --blue: #3452ff;
        --green: #0f9d58;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 30%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
      }
      .wrap {
        max-width: 920px;
        margin: 0 auto;
        padding: 40px 20px 56px;
      }
      .hero {
        border: 1px solid var(--line);
        background: rgba(255, 255, 255, 0.92);
        border-radius: 24px;
        padding: 28px;
        box-shadow: 0 18px 60px rgba(15, 23, 42, 0.06);
      }
      .brand {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 800;
        letter-spacing: 0.02em;
      }
      .brand-mark {
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }
      .badge {
        display: inline-flex;
        align-items: center;
        margin-top: 14px;
        padding: 6px 10px;
        border-radius: 999px;
        border: 1px solid rgba(15, 157, 88, 0.16);
        background: rgba(15, 157, 88, 0.08);
        color: var(--green);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        font-size: 12px;
      }
      h1 {
        margin: 18px 0 0;
        font-size: clamp(42px, 5vw, 64px);
        line-height: 0.96;
        letter-spacing: -0.06em;
      }
      .sub {
        margin-top: 14px;
        color: var(--muted);
        line-height: 1.7;
        max-width: 60ch;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 14px;
        margin-top: 24px;
      }
      .card {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 18px;
      }
      .card-label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .card-value {
        margin-top: 10px;
        font-size: 18px;
        word-break: break-word;
      }
      .actions {
        display: flex;
        gap: 12px;
        flex-wrap: wrap;
        margin-top: 22px;
      }
      a.button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 44px;
        padding: 0 16px;
        border-radius: 12px;
        text-decoration: none;
        font-weight: 600;
      }
      .primary {
        color: white;
        background: linear-gradient(180deg, var(--blue), #1f3fe6);
      }
      .secondary {
        color: var(--text);
        background: var(--surface-2);
        border: 1px solid var(--line);
      }
      .footer {
        margin-top: 18px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.6;
      }
      code {
        background: rgba(52, 82, 255, 0.06);
        border: 1px solid rgba(52, 82, 255, 0.1);
        padding: 2px 6px;
        border-radius: 8px;
        color: var(--text);
      }
      @media (max-width: 720px) {
        .grid { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="hero">
        <div class="brand"><span class="brand-mark"></span> OpenGPU Local Release Preview</div>
        <div class="badge">localhost only • mac-first</div>
        <h1>Signed binaries, local preview, one clean install path.</h1>
        <div class="sub">
          This is the repo-managed localhost release surface used by the current install flow.
          The installer downloads the binary below, verifies the checksum, and then drops you into onboarding.
        </div>

        <div class="actions">
          <a class="button primary" href="$asset_name">Download binary</a>
          <a class="button secondary" href="$asset_name.sha256">View checksum</a>
        </div>

        <div class="grid">
          <div class="card">
            <div class="card-label">Binary</div>
            <div class="card-value"><code>$asset_name</code></div>
          </div>
          <div class="card">
            <div class="card-label">Checksum</div>
            <div class="card-value"><code>$asset_name.sha256</code></div>
          </div>
        </div>

        <div class="footer">
          Use this preview with <code>RELEASE_BASE_URL=$(preview_url) bash install.sh</code>.
        </div>
      </div>
    </div>
  </body>
</html>
EOF

  echo "Prepared local release preview:"
  echo "  root: $preview_root"
  echo "  asset: $asset_name"
  echo "  url:   $(preview_url)"
}

verify_preview() {
  local target asset_name binary_path checksum_path index_path
  target="$(target_triplet)"
  asset_name="${bin_name}-${target}"
  binary_path="$asset_dir/$asset_name"
  checksum_path="$binary_path.sha256"
  index_path="$asset_dir/index.html"

  [ -f "$binary_path" ] || die "missing release binary: $binary_path"
  [ -x "$binary_path" ] || die "release binary is not executable: $binary_path"
  [ -f "$checksum_path" ] || die "missing checksum file: $checksum_path"
  [ -f "$index_path" ] || die "missing release landing page: $index_path"

  "$repo_root/scripts/verify-release-packaging.sh" "$asset_dir" "$asset_name"
  "$repo_root/scripts/release-signing.sh" verify "$asset_dir" "$asset_name"

  echo "Verified local release preview:"
  echo "  asset: $asset_name"
  echo "  root:  $preview_root"
}

server_running() {
  local pid
  [ -f "$server_pid_file" ] || return 1
  pid="$(cat "$server_pid_file" 2>/dev/null || true)"
  [ -n "$pid" ] || return 1
  kill -0 "$pid" >/dev/null 2>&1
}

serve_preview() {
  build_preview

  if server_running; then
    echo "Local release preview already running."
    echo "  pid: $(cat "$server_pid_file")"
    echo "  url: $(preview_url)"
    return 0
  fi

  mkdir -p "$preview_root"
  nohup python3 -m http.server "$port" --bind "$bind_address" --directory "$preview_root" \
    >"$server_log_file" 2>&1 &
  echo $! > "$server_pid_file"

  for _ in $(seq 1 40); do
    if curl -fsS --max-time 1 "$(preview_url)/" >/dev/null 2>&1; then
      echo "Local release preview is live."
      echo "  pid: $(cat "$server_pid_file")"
      echo "  url: $(preview_url)"
      echo "  log: $server_log_file"
      return 0
    fi
    sleep 0.25
  done

  die "release preview server did not become ready; see $server_log_file"
}

stop_preview() {
  if ! [ -f "$server_pid_file" ]; then
    echo "Local release preview is not running."
    return 0
  fi

  local pid
  pid="$(cat "$server_pid_file" 2>/dev/null || true)"
  if [ -n "$pid" ] && kill -0 "$pid" >/dev/null 2>&1; then
    kill "$pid"
    for _ in $(seq 1 20); do
      if ! kill -0 "$pid" >/dev/null 2>&1; then
        rm -f "$server_pid_file"
        echo "Stopped local release preview."
        return 0
      fi
      sleep 0.1
    done
    kill -9 "$pid" >/dev/null 2>&1 || true
  fi

  rm -f "$server_pid_file"
  echo "Stopped local release preview."
}

status_preview() {
  if server_running; then
    echo "running"
    echo "  pid: $(cat "$server_pid_file")"
    echo "  url: $(preview_url)"
    return 0
  fi

  echo "stopped"
}

clean_preview() {
  stop_preview || true
  rm -rf "$preview_root"
  echo "Removed preview tree: $preview_root"
}

case "${1:-up}" in
  build) build_preview ;;
  verify) verify_preview ;;
  serve|up) serve_preview ;;
  stop) stop_preview ;;
  status) status_preview ;;
  clean) clean_preview ;;
  url) preview_url ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 1 ;;
esac
