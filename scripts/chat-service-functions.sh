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
