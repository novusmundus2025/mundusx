#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

assert_contains() {
  local file="$1"
  local text="$2"

  if ! rg -F -q "$text" "$file"; then
    echo "expected $file to contain: $text" >&2
    exit 1
  fi
}

assert_not_contains() {
  local file="$1"
  local text="$2"

  if rg -F -q "$text" "$file"; then
    echo "did not expect $file to contain: $text" >&2
    exit 1
  fi
}

assert_contains "apps/cli/README.md" "# NovusX CLI"
assert_contains "apps/cli/README.md" 'NovusX is the product; `opengpu` is the current CLI command.'
assert_not_contains "apps/cli/README.md" "# opengpu CLI"

assert_contains "docs/install-page.md" "This page defines the localhost-first install touch for NovusX"
assert_contains "docs/install-page.md" "explain that NovusX installs with one command"
assert_contains "docs/install-page.md" 'the `opengpu` CLI is installed with one command'
assert_not_contains "docs/install-page.md" "This page defines the localhost-first touch for `opengpu`"

echo "branding surface looks consistent"
