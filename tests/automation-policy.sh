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

assert(policy.repository_type === "cli-distribution", "repository_type must be cli-distribution");
assert(policy.success_verification?.requires_merge_to_uat === true, "requires_merge_to_uat must be true");
assert(policy.success_verification?.requires_deployment_url === false, "cli repo must not require deployment_url");

const requiredChecks = policy.success_verification?.post_merge_checks || [];
assert(requiredChecks.includes("repo_validations"), "post_merge_checks must include repo_validations");
assert(requiredChecks.includes("artifact_or_smoke_verification"), "post_merge_checks must include artifact_or_smoke_verification");

const blockerRules = policy.blocker_rules || {};
assert(Array.isArray(blockerRules.hard_blockers), "hard_blockers must be an array");
assert(blockerRules.hard_blockers.includes("failing_validations"), "hard_blockers must include failing_validations");
assert(blockerRules.hard_blockers.includes("ambiguous_issue_selection"), "hard_blockers must include ambiguous_issue_selection");

assert(Array.isArray(blockerRules.non_blockers_for_cli_repos), "non_blockers_for_cli_repos must be an array");
assert(blockerRules.non_blockers_for_cli_repos.includes("missing_uat_service_url"), "CLI non-blockers must include missing_uat_service_url");
assert(blockerRules.non_blockers_for_cli_repos.includes("missing_live_deployment_healthcheck"), "CLI non-blockers must include missing_live_deployment_healthcheck");

console.log("automation policy matches CLI repo expectations");
EOF
