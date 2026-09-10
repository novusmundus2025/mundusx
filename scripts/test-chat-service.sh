#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home space/bin" "$scratch/mock"
export INSTALL_DIR="$scratch/home space/bin"
export MUNDUSX_HOME="$scratch/home space/data"
export XDG_CONFIG_HOME="$scratch/config" XDG_DATA_HOME="$scratch/share"
export MUNDUSX_CHAT_SERVICE_ACTIVATE=0
export TEST_CALLS="$scratch/calls"
printf '#!/usr/bin/env bash\nprintf "connector:%%s\\n" "$*" >> "$TEST_CALLS"\n' > "$INSTALL_DIR/mundusx"
chmod +x "$INSTALL_DIR/mundusx"
platform="${MUNDUSX_SERVICE_PLATFORM:-$(uname -s)}"
export MUNDUSX_SERVICE_PLATFORM="$platform"
env HOME="$scratch/home space" bash "$root/install.sh" --configure-chat-service
# Installation is repeatable, including macOS bundle metadata.
mkdir -p "$MUNDUSX_HOME"
printf '{"test":"saved pairing"}\n' > "$MUNDUSX_HOME/chat-connection.json"
cp "$MUNDUSX_HOME/chat-connection.json" "$scratch/pairing-before"
env HOME="$scratch/home space" bash "$root/install.sh" --configure-chat-service
cmp "$scratch/pairing-before" "$MUNDUSX_HOME/chat-connection.json"
if "$MUNDUSX_HOME/bin/mundusx-reconnect" 'mundusx://execute?command=bad'; then
  echo 'Unexpected URI accepted' >&2; exit 1
fi
if [ "$platform" = Linux ]; then
  printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$*" >> "$TEST_CALLS"\n' > "$scratch/mock/systemctl"
  chmod +x "$scratch/mock/systemctl"
  for attempt in 1 2; do
    PATH="$scratch/mock:$PATH" "$MUNDUSX_HOME/bin/mundusx-reconnect" mundusx://reconnect
  done
  [ "$(grep -c '^--user start mundusx-chat.service$' "$TEST_CALLS")" = 2 ]
  grep -q 'Restart=on-failure' "$XDG_CONFIG_HOME/systemd/user/mundusx-chat.service"
  grep -q 'x-scheme-handler/mundusx' "$XDG_DATA_HOME/applications/mundusx-reconnect.desktop"
else
  plutil -lint "$scratch/home space/Library/LaunchAgents/ai.mundusx.chat.plist"
  [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleURLTypes:0:CFBundleURLSchemes:0' "$scratch/home space/Applications/MundusX Reconnect.app/Contents/Info.plist")" = mundusx ]
  ! grep -q 'kickstart -k' "$MUNDUSX_HOME/bin/mundusx-reconnect"
fi
"$MUNDUSX_HOME/bin/mundusx-chat-service"
grep -q '^connector:connect$' "$TEST_CALLS"
rm "$MUNDUSX_HOME/chat-connection.json" "$TEST_CALLS"
"$MUNDUSX_HOME/bin/mundusx-chat-service"
[ ! -e "$TEST_CALLS" ]
sed -n '/^# BEGIN MUNDUSX CHAT SERVICE$/,/^# END MUNDUSX CHAT SERVICE$/p' "$root/install.sh" > "$scratch/embedded"
cmp "$root/scripts/chat-service-functions.sh" "$scratch/embedded"
echo "Chat service checks passed ($platform)"
