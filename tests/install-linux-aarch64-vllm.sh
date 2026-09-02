#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

bin_dir="$tmp_dir/bin"
install_dir="$tmp_dir/install"
opengpu_home="$tmp_dir/opengpu-home"
mkdir -p "$bin_dir" "$install_dir"

cat >"$bin_dir/uname" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  -s) printf 'Linux\n' ;;
  -m) printf 'aarch64\n' ;;
  *) printf 'Linux\n' ;;
esac
EOF

cat >"$bin_dir/curl" <<'EOF'
#!/usr/bin/env bash
out=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
printf '%s\n' "$url" >>"$TEST_DOWNLOAD_LOG"
if [[ "$url" == *.sha256 ]]; then
  printf 'abc123  %s\n' "$(basename "${url%.sha256}")" >"$out"
else
  printf 'placeholder\n' >"$out"
fi
EOF

cat >"$bin_dir/shasum" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

cat >"$bin_dir/nvidia-smi" <<'EOF'
#!/usr/bin/env bash
printf 'NVIDIA GB10\n'
EOF

cat >"$bin_dir/nvidia-ctk" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

cat >"$bin_dir/docker" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$TEST_DOCKER_LOG"
exit 0
EOF

chmod +x "$bin_dir"/*
export PATH="$bin_dir:$PATH"
export TEST_DOWNLOAD_LOG="$tmp_dir/download.log"
export TEST_DOCKER_LOG="$tmp_dir/docker.log"

output="$(
  INSTALL_DIR="$install_dir" \
  OPENGPU_HOME="$opengpu_home" \
  OPENGPU_SKIP_INSTALL_SMOKE=1 \
  bash "$repo_root/install.sh" --install-only 2>&1
)"

printf '%s\n' "$output" | grep -F "Detected NVIDIA GB10/GX10" >/dev/null
printf '%s\n' "$output" | grep -F "target: aarch64-unknown-linux-gnu" >/dev/null
printf '%s\n' "$output" | grep -F "  opengpu install" >/dev/null
printf '%s\n' "$output" | grep -F "  opengpu start" >/dev/null
grep -F "opengpu-aarch64-unknown-linux-gnu" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "opengpu-node-agent-aarch64-unknown-linux-gnu" "$TEST_DOWNLOAD_LOG" >/dev/null
grep -F "run --rm --gpus all nvcr.io/nvidia/cuda:13.0.1-base-ubuntu24.04 nvidia-smi" "$TEST_DOCKER_LOG" >/dev/null
grep -F "pull nvcr.io/nvidia/vllm@sha256:63b808804826a028e38f559747a9e4d5985cf676616fbaa70c1937c58f83e13e" "$TEST_DOCKER_LOG" >/dev/null
grep -F "VLLM_GPU_MEMORY_UTILIZATION=0.70" "$opengpu_home/runtimes/vllm/runtime.conf" >/dev/null
grep -F "VLLM_MAX_NUM_SEQS=4" "$opengpu_home/runtimes/vllm/runtime.conf" >/dev/null
grep -F "VLLM_START_TIMEOUT_SECONDS=1800" "$opengpu_home/runtimes/vllm/runtime.conf" >/dev/null

echo "PASS: install.sh auto-detects GB10 and provisions the pinned Linux ARM64 vLLM runtime"
