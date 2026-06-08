#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

policy_file=".github/priority-label-policy.json"
doc_file="docs/priority-label-policy.md"
workflow_file=".github/workflows/mundusx-project-status.yml"

if [[ ! -f "$policy_file" ]]; then
  echo "expected $policy_file to exist" >&2
  exit 1
fi

if [[ ! -f "$doc_file" ]]; then
  echo "expected $doc_file to exist" >&2
  exit 1
fi

if [[ ! -f "$workflow_file" ]]; then
  echo "expected $workflow_file to exist" >&2
  exit 1
fi

node <<'EOF'
const fs = require("fs");

const policy = JSON.parse(fs.readFileSync(".github/priority-label-policy.json", "utf8"));
const doc = fs.readFileSync("docs/priority-label-policy.md", "utf8");
const workflow = fs.readFileSync(".github/workflows/mundusx-project-status.yml", "utf8");

function assert(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

assert(policy.version === 1, "policy version must be 1");
assert(policy.project_number === 3, "project_number must track GitHub Project 3");
assert(Array.isArray(policy.priority_order), "priority_order must be an array");
assert(
  JSON.stringify(policy.priority_order) === JSON.stringify(["P0", "P1", "P2", "P3"]),
  "priority_order must stay P0 -> P3"
);

const allowedAreas = ["Public", "Private", "Platform", "Workflow", "Docs"];
assert(
  JSON.stringify(policy.allowed_areas) === JSON.stringify(allowedAreas),
  "allowed_areas must match the board areas"
);

const mappings = policy.priority_labels || {};
const expectedMappings = {
  P0: "priority:P0",
  P1: "priority:P1",
  P2: "priority:P2",
  P3: "priority:P3",
};

for (const [priority, label] of Object.entries(expectedMappings)) {
  assert(mappings[priority] === label, `priority label for ${priority} must be ${label}`);
  assert(doc.includes(`\`${label}\``), `docs must mention label ${label}`);
}

assert(policy.selection_rule?.require_exactly_one_priority_label === true, "selection rule must require exactly one priority label");
assert(policy.selection_rule?.fallback_to_title_prefix === true, "selection rule must allow title-prefix fallback");
assert(policy.selection_rule?.highest_priority_first === true, "selection rule must require highest-priority-first execution");
assert(policy.selection_rule?.blocked_items_must_not_be_selected === true, "selection rule must exclude blocked items");

assert(Array.isArray(policy.title_prefixes), "title_prefixes must be an array");
assert(
  JSON.stringify(policy.title_prefixes) === JSON.stringify(["[P0]", "[P1]", "[P2]", "[P3]"]),
  "title_prefixes must mirror the priority labels"
);

assert(doc.includes("GitHub Project 3"), "docs must mention GitHub Project 3");
assert(doc.includes("exactly one"), "docs must explain the exactly-one-label rule");
assert(workflow.includes('PROJECT_NUMBER: "3"'), "workflow must target GitHub Project 3");
assert(workflow.includes("organization(login: $owner)") || workflow.includes("organization(login:$owner)"), "workflow must support organization-owned projects");

console.log("priority label policy matches repo expectations");
EOF
