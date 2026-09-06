#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

policy_file=".github/automation-policy.json"

if [[ ! -f "$policy_file" ]]; then
  echo "expected $policy_file to exist" >&2
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

const issueLinkPolicy = policy.issue_link_policy || {};
assert(
  issueLinkPolicy.source_prefix === "Source:",
  "issue_link_policy.source_prefix must be Source:",
);
assert(
  issueLinkPolicy.required_notion_host === "https://www.notion.so/",
  "issue_link_policy.required_notion_host must be https://www.notion.so/",
);
assert(
  issueLinkPolicy.github_reference_format === "owner/repo#issue-number",
  "issue_link_policy.github_reference_format must be owner/repo#issue-number",
);
EOF

require_line() {
  local path="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$path"; then
    echo "missing pattern '$pattern' in $path" >&2
    exit 1
  fi
}

require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "Source: [Related spec or backlog page](https://www.notion.so/"
require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "Source: [Related spec or backlog page](https://www.notion.so/"

echo "Notion issue link policy verified."
