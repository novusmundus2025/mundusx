#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

workflow_file=".github/workflows/mundusx-project-status.yml"
policy_file=".github/automation-policy.json"

if [[ ! -f "$workflow_file" ]]; then
  echo "expected $workflow_file to exist" >&2
  exit 1
fi

if [[ ! -f "$policy_file" ]]; then
  echo "expected $policy_file to exist" >&2
  exit 1
fi

node <<'EOF'
const fs = require("fs");

const workflow = fs.readFileSync(".github/workflows/mundusx-project-status.yml", "utf8");
const policy = JSON.parse(fs.readFileSync(".github/automation-policy.json", "utf8"));

function assert(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

assert(
  workflow.includes('PROJECT_NUMBER: "3"'),
  "mundusx-project-status workflow must target GitHub Project 3",
);

assert(
  workflow.includes("organization(login: $owner)"),
  "mundusx-project-status workflow must load project metadata from the mundusx organization",
);

const priorityPolicy = policy.issue_priority_policy || {};
assert(
  Array.isArray(priorityPolicy.allowed_labels),
  "issue_priority_policy.allowed_labels must be an array",
);

for (const label of ["priority:P0", "priority:P1", "priority:P2"]) {
  assert(
    priorityPolicy.allowed_labels.includes(label),
    `issue_priority_policy.allowed_labels must include ${label}`,
  );
}

assert(
  priorityPolicy.default_label === "priority:P1",
  "issue_priority_policy.default_label must be priority:P1",
);

assert(
  priorityPolicy.source_of_truth === "GitHub Project 3",
  "issue_priority_policy.source_of_truth must be GitHub Project 3",
);

console.log("project status workflow and priority policy match GitHub Project 3 expectations");
EOF
