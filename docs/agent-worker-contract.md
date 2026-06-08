# Agent / Worker Contract

This page defines the interface the CLI, node agent, and worker should share when the next phase starts.

## Goal

Keep the CLI focused on user setup and machine readiness, while the agent handles background presence and the worker handles the actual local compute.

## Roles

- `CLI`: user-facing setup, start, status, and exit
- `Agent`: background service on the provider machine
- `Worker`: short-lived process that performs a job locally

## Registration

When the agent comes online, it should register with:

- node ID
- hostname
- public key fingerprint
- public key hex
- resolved backend
- contribution percent
- agent version

## Heartbeat

The agent should periodically send a heartbeat that includes:

- node ID
- backend
- agent state
- available memory
- available GPU percent
- contribution percent
- worker health snapshot
- timestamp

The control plane uses that heartbeat to decide whether the node is:

- ready
- busy
- paused
- offline

## Worker Launch

When a job arrives, the agent should launch a worker locally with:

- job ID
- node ID
- backend
- prompt or task payload
- optional model name
- optional system prompt
- optional max token count
- optional temperature
- optional top-p
- optional seed

The worker should return:

- job ID
- worker ID
- status
- output text
- resolved backend
- node ID
- optional error text
 
When running on Mac `M`, the worker uses the cached GGUF model with `llama.cpp` in single-turn batch mode via `llama-cli --device BLAS`.
It should keep the default load modest so the contributor machine stays responsive, and it should return only the final answer text instead of the full runtime transcript.

## Job Queue And Completion

The current prototype adds one small control-plane queue:

1. A client, SDK, or dashboard submits a job to `POST /v1/jobs`.
2. The control plane stores the job as `queued`.
3. The agent asks `GET /v1/jobs/next?node_id=...` for work.
4. If a queued job matches the node backend, the control plane marks it `assigned`.
5. The agent launches the worker locally.
6. The worker result is posted back to `POST /v1/jobs/complete`.
7. The control plane marks the job `completed` or `failed`.

## Output Path

1. Client sends a request to the control plane.
2. Control plane queues the job.
3. Agent claims the job when it is ready.
4. Agent launches the worker locally.
5. Worker runs on `M` series using the local Mac runtime.
6. Worker returns output to the agent.
7. Agent forwards the result upstream.

## What The CLI Needs To Be Ready For

- show the resolved backend cleanly
- expose the device identity and public key fingerprint
- preserve contribution percent and pause state
- keep room for local agent readiness checks
- avoid mixing demo node inventory into the main status view

## Status Of This Contract

The data shapes are now defined in:

- `apps/cli/src/types.rs`
- `packages/proto/schema/opengpu.proto`

The transport and service implementations are partially in place, including:

- agent registration and heartbeat transport to the control plane with device signatures
- control-plane job queue submission and claim flow
- local worker launch from the agent

The device keypair is now the node identity layer. The control plane verifies the node's signed requests instead of requiring a separate contributor login for the machine itself. The hostname and `identityTrustPath` are part of the signed contributor identity so the operator can see which physical machine is represented and whether it is using Keychain or the local encrypted fallback without relying on an unauthenticated label.

The health check command now verifies the local Mac runner without starting a full job, and it also reports whether the current contribution cap is allowed by the Mac power state. The agent now converts that policy into paused heartbeats so the control plane will not assign work when the Mac should stay quiet. The same heartbeat also carries the worker health snapshot so the control plane can show readiness before it assigns work. The worker launch payload now carries the execution profile too, so the control plane can hand the worker a real system prompt, max token count, temperature, top-p, and seed instead of only a bare prompt string.
