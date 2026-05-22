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
- public key fingerprint
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

The worker should return:

- job ID
- worker ID
- status
- optional error text

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
5. Worker runs on `M` series.
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

- agent registration and heartbeat transport to the control plane
- control-plane job queue submission and claim flow
- local worker launch from the agent

The next gap is replacing the simulated worker output with real execution.
The current worker already performs deterministic local compute; the next gap is a model-aware `M` inference kernel or model execution path.
