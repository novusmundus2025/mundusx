#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

node <<'EOF'
const fs = require("fs");

function assertContains(path, snippets) {
  const text = fs.readFileSync(path, "utf8");
  for (const snippet of snippets) {
    if (!text.includes(snippet)) {
      throw new Error(`${path} is missing: ${snippet}`);
    }
  }
}

assertContains("agents/node/src/contracts.rs", [
  "pub public_key_hex: String",
  "pub identity_trust_path: String",
  "pub worker_health: WorkerHealthReport",
  "pub power_source: String",
  "pub output: String",
  "pub backend: Backend",
  "pub node_id: String",
]);

assertContains("apps/cli/src/types.rs", [
  "pub public_key_hex:",
  "pub identity_trust_path:",
  "pub worker_health:",
  "pub power_source:",
  "pub output:",
  "pub backend:",
  "pub node_id:",
]);

assertContains("packages/proto/schema/opengpu.proto", [
  "string public_key_hex =",
  "string identity_trust_path =",
  "WorkerHealthReport worker_health =",
  "string power_source =",
  "string output =",
  "Backend backend =",
  "string node_id =",
]);

assertContains("docs/agent-worker-contract.md", [
  "public key hex",
  "identityTrustPath",
  "worker health snapshot",
  "The worker should return:",
  "optional error text",
]);

console.log("shared contract surface is aligned");
EOF
