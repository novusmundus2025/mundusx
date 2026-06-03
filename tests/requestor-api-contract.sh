#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

requestor_doc="docs/requestor-api.md"
gap_doc="docs/gap-checklist.md"

if [[ ! -f "$requestor_doc" ]]; then
  echo "expected $requestor_doc to exist" >&2
  exit 1
fi

if [[ ! -f "$gap_doc" ]]; then
  echo "expected $gap_doc to exist" >&2
  exit 1
fi

node <<'EOF'
const fs = require("fs");

const requestorDoc = fs.readFileSync("docs/requestor-api.md", "utf8");
const gapDoc = fs.readFileSync("docs/gap-checklist.md", "utf8");

function assert(condition, message) {
  if (!condition) {
    console.error(message);
    process.exit(1);
  }
}

assert(
  requestorDoc.includes("## `GET /v1/models`"),
  "requestor API doc must define the GET /v1/models contract",
);

assert(
  requestorDoc.includes("## Streaming Responses"),
  "requestor API doc must define streaming responses",
);

assert(
  requestorDoc.includes("## Retry, Timeout, And Idempotency"),
  "requestor API doc must define retry, timeout, and idempotency rules",
);

assert(
  requestorDoc.includes("Idempotency-Key"),
  "requestor API doc must mention the Idempotency-Key header",
);

assert(
  requestorDoc.includes("X-Request-Timeout-Ms"),
  "requestor API doc must mention the timeout override header",
);

assert(
  requestorDoc.includes("text/event-stream"),
  "requestor API doc must describe the streaming transport",
);

assert(
  requestorDoc.includes("## Compatibility Surface"),
  "requestor API doc must summarize the supported compatibility surface",
);

assert(
  gapDoc.includes("- requestor API completion"),
  "gap checklist should keep the requestor API completion feature grouped explicitly",
);

assert(
  gapDoc.includes("  - documented `GET /v1/models` contract"),
  "gap checklist should record GET /v1/models as completed",
);

assert(
  gapDoc.includes("  - documented streaming response contract"),
  "gap checklist should record streaming as completed",
);

assert(
  gapDoc.includes("  - documented retry / timeout / idempotency rules"),
  "gap checklist should record retry/timeout/idempotency as completed",
);

console.log("requestor API contract documentation is complete");
EOF
