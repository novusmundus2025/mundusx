# IDE-owned native tool workflow

OpenAI-compatible tool turns use the IDE's workflow by default. The contributor
runs inference; it does not execute the IDE's tools or claim filesystem authority.

The worker preserves the host's history, tool names, schemas, execution inputs,
and output budget. It does not substitute reads for proposed edits, fabricate
validation calls, or require a particular file-edit tool. Existing-file errors,
approvals, checkpoints and recovery belong to the IDE and its tool implementations.

Wire validation still rejects incomplete JSON, unknown functions, invalid required
arguments and violations of explicit tool choice. Missing terminal display labels
may be filled only when declared by the host schema; command and execution settings
are preserved. A malformed argument response has at most one schema repair attempt.
No failed call is executed or reported as successful by the contributor.

For diagnosing older integrations only, `OPENGPU_NATIVE_WORKFLOW=legacy` enables
the previous contributor workflow adapter. It is not the default API behavior.

Model architecture support, valid streaming and contributor isolation remain
required. An inference-only contributor never gains access to the caller's files.

When an IDE omits `max_tokens`, the worker forwards the job output budget to MLX (4,096 only if no job budget exists). Explicit caller limits are preserved. This avoids MLX’s 512-token default truncating ordinary file writes.

The public gateway assigns native IDE turns an 8,192-token output allowance only when the caller omits a limit. Explicit limits remain unchanged. The worker reports `finish_reason: length` as output truncation, releases no tool calls, and does not attempt a same-budget schema repair.
