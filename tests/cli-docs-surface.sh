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

assert_contains "docs/cli-commands.md" 'download an official open preset first and then activate it'
assert_contains "docs/cli-commands.md" 'downloading an official open preset first when available'
assert_contains "docs/cli-commands.md" 'download official open presets from the reviewed catalog before caching or activating them'
assert_not_contains "docs/cli-commands.md" 'real model downloads are still a future step'

echo "cli docs match current model download behavior"
