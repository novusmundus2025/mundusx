#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$repo_root/$path" ]]; then
    echo "missing file: $path" >&2
    exit 1
  fi
}

require_line() {
  local path="$1"
  local pattern="$2"
  if ! grep -Fq "$pattern" "$repo_root/$path"; then
    echo "missing pattern '$pattern' in $path" >&2
    exit 1
  fi
}

require_file ".github/ISSUE_TEMPLATE/bug_report.yml"
require_file ".github/ISSUE_TEMPLATE/feature_request.yml"
require_file ".github/ISSUE_TEMPLATE/spec_request.yml"
require_file ".github/ISSUE_TEMPLATE/config.yml"

require_line ".github/ISSUE_TEMPLATE/config.yml" "blank_issues_enabled: false"
require_line ".github/ISSUE_TEMPLATE/config.yml" "contact_links:"

require_line ".github/ISSUE_TEMPLATE/bug_report.yml" "name: Bug report"
require_line ".github/ISSUE_TEMPLATE/bug_report.yml" "title: \"[Bug]: \""
require_line ".github/ISSUE_TEMPLATE/bug_report.yml" "id: summary"
require_line ".github/ISSUE_TEMPLATE/bug_report.yml" "id: steps"
require_line ".github/ISSUE_TEMPLATE/bug_report.yml" "id: expected"
require_line ".github/ISSUE_TEMPLATE/bug_report.yml" "id: environment"

require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "name: Feature request"
require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "title: \"[Feature]: \""
require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "id: problem"
require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "id: proposal"
require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "id: acceptance"
require_line ".github/ISSUE_TEMPLATE/feature_request.yml" "id: scope"

require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "name: Spec request"
require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "title: \"[Spec]: \""
require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "id: problem"
require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "id: requirements"
require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "id: acceptance"
require_line ".github/ISSUE_TEMPLATE/spec_request.yml" "id: references"

echo "GitHub issue templates verified."
