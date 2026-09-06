#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

policy_file="docs/macos-identity-policy.md"
lifecycle_file="docs/device-identity-lifecycle.md"
master_file="docs/master-checklist.md"

for file in "$policy_file" "$lifecycle_file" "$master_file"; do
  if [[ ! -f "$file" ]]; then
    echo "expected $file to exist" >&2
    exit 1
  fi
done

node <<'EOF'
const fs = require("fs");

const policy = fs.readFileSync("docs/macos-identity-policy.md", "utf8");
const lifecycle = fs.readFileSync("docs/device-identity-lifecycle.md", "utf8");
const master = fs.readFileSync("docs/master-checklist.md", "utf8");

function assert(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

for (const term of [
  "Non-exportable OS-backed signing",
  "Keychain encrypted fallback",
  "Local encrypted fallback",
  "local-encrypted-fallback",
  "enterprise",
  "production",
  "identityTrustPath",
]) {
  assert(policy.includes(term), `macOS identity policy must mention ${term}`);
}

assert(
  policy.includes("Production rollout remains gated"),
  "macOS identity policy must state the production gate",
);

assert(
  lifecycle.includes("docs/macos-identity-policy.md"),
  "device identity lifecycle must link to the macOS identity policy",
);

assert(
  master.includes("[x] macOS non-exportable identity enforcement policy"),
  "master checklist must mark macOS identity enforcement policy complete",
);

console.log("macOS identity policy docs cover required modes and enforcement posture");
EOF
