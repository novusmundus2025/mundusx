#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

assert_json_license() {
  local file="$1"
  local expected="$2"

  local actual
  actual="$(node -e 'const fs=require("fs"); const file=process.argv[1]; const data=JSON.parse(fs.readFileSync(file,"utf8")); process.stdout.write(data.license ?? "");' "$file")"

  if [[ "$actual" != "$expected" ]]; then
    echo "expected $file license to be $expected, got ${actual:-<missing>}" >&2
    exit 1
  fi
}

assert_toml_license() {
  local file="$1"
  local expected="$2"

  if ! rg -q "^license = \"$expected\"$" "$file"; then
    echo "expected $file license to declare $expected" >&2
    exit 1
  fi
}

if ! rg -q "Apache License" LICENSE; then
  echo "expected root LICENSE to contain Apache License text" >&2
  exit 1
fi

assert_json_license "package.json" "Apache-2.0"
assert_json_license "packages/shared/package.json" "Apache-2.0"
assert_json_license "packages/proto/package.json" "Apache-2.0"
assert_json_license "workers/m-series/package.json" "Apache-2.0"
assert_json_license "workers/cuda/package.json" "Apache-2.0"

assert_toml_license "apps/cli/Cargo.toml" "Apache-2.0"
assert_toml_license "agents/node/Cargo.toml" "Apache-2.0"

echo "license surface looks publish-safe"
