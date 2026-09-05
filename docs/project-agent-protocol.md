# MundusX project agent protocol

Status: Phase 1 contract

Protocol version: `mundusx-project-agent/v1`

This protocol is used only when a signed-in user attaches a local project to a
conversation. Conversations without a project use ordinary MundusX Chat and
MUST NOT require, start, or advertise an agent harness.

The protocol is language- and framework-agnostic. A Node.js project is one test
fixture, not a restriction. The runner discovers and uses tools available in
the approved project workspace.

## Components and trust boundaries

1. The browser creates an asynchronous project task in Chat.
2. The authenticated MundusX runner claims that task.
3. The runner starts a loopback-only model proxy and launches the selected
   harness, initially Hermes, inside the resolved project directory.
4. Hermes sends OpenAI-compatible model turns to the loopback proxy.
5. The proxy submits each turn as a short remote model job and polls it.
6. Chat routes the job through the EHDA agnostic model and returns either one
   tool call or a final assistant response.
7. Hermes executes tools locally and the runner publishes sanitized progress.

The Chat connector credential MUST remain owned by the runner. It MUST NOT be
placed in the Hermes environment. Hermes receives a random, per-task loopback
credential that expires when the task ends.

## Model-job API

All routes require the connector bearer credential and verify ownership of the
parent project task.

### Submit

`POST /api/agent/model/jobs`

The request follows `project-agent-v1.schema.json#/$defs/modelJobSubmit`. A
successful submission returns HTTP 202 and a stable `job_id`; it never waits
for model completion.

### Poll

`GET /api/agent/model/jobs/{job_id}`

Returns one of `queued`, `running`, `completed`, `failed`, `cancelled`, or
`expired`. Completed results use the OpenAI chat-completion message shape and
contain either exactly one `tool_calls` entry or final textual content.

Clients use bounded exponential backoff starting at 250 ms and capped at two
seconds. `retry_after_ms` overrides the next interval when present.

### Cancel

`POST /api/agent/model/jobs/{job_id}/cancel`

Cancellation is idempotent. Cancelling the parent project task also cancels
all of its non-terminal model jobs and terminates the local Hermes process.

## State and lifecycle

Allowed transitions are:

```text
queued -> running -> completed
queued -> cancelled
running -> cancelled
queued -> failed
running -> failed
queued -> expired
running -> expired
```

Terminal states never transition. Model jobs have a server-defined expiry and
are scoped to `account_id`, `connection_id`, `project_task_id`, and
`project_slug`. Cross-account or cross-connector access returns 404.

The runner may safely retry submission with the same `idempotency_key`.
Duplicate keys within the same parent task return the original job.

## Project boundary and permissions

- `project_slug` is a normalized relative directory beneath the approved
  connector workspace. Absolute paths and traversal are rejected.
- The canonical task directory is the complete filesystem boundary.
- Read-only tasks do not imply mutation permission.
- File writes and commands require the mutation grant recorded on the parent
  project task.
- Contributor mode is independent and MUST remain disabled unless separately
  enabled by the user.

## Progress events

The runner reports sanitized events with monotonic sequence numbers:

- `harness_started`
- `model_turn_queued`
- `model_turn_completed`
- `tool_started`
- `tool_completed`
- `file_changed`
- `verification_started`
- `verification_completed`
- `task_completed`
- `task_failed`

Secrets, full environment variables, connector credentials, and unrestricted
command output are never included. Replayed sequence numbers are idempotent.

## Compatibility

The runner advertises `mundusx-project-agent/v1`. Chat rejects an unsupported
major version with `upgrade_required` and may accept additive minor fields.
Unknown response fields are ignored. Unknown states, tool result variants, or
major versions fail closed.

