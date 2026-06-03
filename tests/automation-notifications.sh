#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

policy_file=".github/automation-policy.json"
doc_file="docs/automation-policy.md"

if [[ ! -f "$policy_file" ]]; then
  echo "expected $policy_file to exist" >&2
  exit 1
fi

if [[ ! -f "$doc_file" ]]; then
  echo "expected $doc_file to exist" >&2
  exit 1
fi

node <<'EOF'
const fs = require("fs");

const policy = JSON.parse(fs.readFileSync(".github/automation-policy.json", "utf8"));

function assert(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

const terminalNotifications = policy.terminal_notifications || {};

assert(
  Array.isArray(terminalNotifications.success_channels),
  "terminal_notifications.success_channels must be an array",
);
assert(
  terminalNotifications.success_channels.includes("codex_inbox"),
  "terminal_notifications.success_channels must include codex_inbox",
);
assert(
  terminalNotifications.success_channels.includes("email"),
  "terminal_notifications.success_channels must include email",
);

assert(
  Array.isArray(terminalNotifications.blocker_channels),
  "terminal_notifications.blocker_channels must be an array",
);
assert(
  terminalNotifications.blocker_channels.includes("codex_inbox"),
  "terminal_notifications.blocker_channels must include codex_inbox",
);
assert(
  terminalNotifications.blocker_channels.includes("email"),
  "terminal_notifications.blocker_channels must include email",
);

assert(
  terminalNotifications.blocker_email_after === "hard_blocker_or_retry_window_exhausted",
  "terminal_notifications.blocker_email_after must require a hard blocker or exhausted retries",
);
assert(
  terminalNotifications.gmail_unavailable_treated_as_blocker === true,
  "terminal_notifications.gmail_unavailable_treated_as_blocker must be true",
);
assert(
  terminalNotifications.transient_retry_window_minutes === 30,
  "terminal_notifications.transient_retry_window_minutes must be 30",
);

console.log("automation notification policy verified");
EOF

require_line() {
  local path="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$path"; then
    echo "missing pattern '$pattern' in $path" >&2
    exit 1
  fi
}

require_line "$doc_file" "## Terminal Notifications"
require_line "$doc_file" "success outcome must send both a Codex inbox notification and an email"
require_line "$doc_file" "blocker outcome must send both a Codex inbox notification and an email"
require_line "$doc_file" "retry for up to 30 minutes total before sending the blocker email"
require_line "$doc_file" "If Gmail is unavailable, treat that as part of the blocker itself."

echo "automation notification docs verified."
