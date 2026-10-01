#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/assets" "$tmp/bin" "$tmp/install"
cat > "$tmp/bin/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in -s) echo Linux;; -m) echo x86_64;; esac
EOF
chmod +x "$tmp/bin/uname"
for name in opengpu opengpu-node-agent mundusx mundusx-agent-server; do
  asset="$name-x86_64-unknown-linux-gnu"
  cat > "$tmp/assets/$asset" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$TEST_CLI_LOG"
if [[ "$*" == *"--connection pair"* ]]; then exit 2; fi
EOF
  chmod +x "$tmp/assets/$asset"
  (cd "$tmp/assets" && sha256sum "$asset" > "$asset.sha256")
done
export TEST_CLI_LOG="$tmp/cli.log" MUNDUSX_SKIP_CHAT_SERVICE=1
export INSTALL_DIR="$tmp/install" OPENGPU_GLOBAL_BIN_DIR="$tmp/bin"
export PATH="$tmp/bin:$PATH"
bash "$repo/install.sh" --local-assets "$tmp/assets" --connection direct \
  --cluster-url http://127.0.0.1:1234/v1 --cluster-model 'model with space' \
  --auto-start --cap-percent 50 --max-jobs 1 > "$tmp/install.log"
grep -Fx 'install --public --cap-percent 50 --max-jobs 1 --connection direct --cluster-url http://127.0.0.1:1234/v1 --cluster-model model with space' "$TEST_CLI_LOG"
grep -Fx 'start --background --max-jobs 1' "$TEST_CLI_LOG"
: > "$TEST_CLI_LOG"
if bash "$repo/install.sh" --local-assets "$tmp/assets" --connection pair \
  --cluster-url http://127.0.0.1:11434 --auto-start --max-jobs 1 > "$tmp/pair.log"; then
  echo 'PAIR gate must stop bootstrap' >&2; exit 1
fi
if grep -q '^start\|^onboarding' "$TEST_CLI_LOG"; then
  echo 'PAIR bootstrap must not start or complete onboarding' >&2; exit 1
fi
echo 'PASS: direct options forwarded and PAIR failure stops startup'
