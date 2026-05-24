# Worker

The worker is the short-lived process that performs a single unit of compute on the provider machine.

## Prototype Shape

In the current scaffold, the worker runs as a subcommand of the node agent binary.

## Current Worker Entry

- `opengpu-agent worker` - run one local worker job and print the result
- `opengpu-agent launch-worker` - ask the agent to spawn the worker subprocess for a demo job

## Worker Input

- job ID
- node ID
- backend
- prompt
- optional model
- optional system prompt
- optional max token count
- optional temperature
- optional top-p
- optional seed

## Worker Output

- job ID
- worker ID
- status
- backend
- node ID
- output text
- optional error

## Current Behavior

The prototype worker:

- performs real local inference on macOS `M` by calling `llama.cpp` against the cached GGUF model
- runs in single-turn batch mode via `llama-cli --device BLAS`
- uses conservative defaults to stay quieter on the contributor machine:
  - lower token count
  - low thread count
  - no perf logging
- honors the execution profile from the job payload when it is present:
  - system prompt is prepended to the prompt
  - max tokens maps to the generation length
  - temperature, top-p, and seed flow through to `llama-cli`
- returns only the generated answer text from the local model
- chooses the Mac `M` path when `auto` is passed on Apple Silicon
- is normally launched by the node agent, not run directly by users
- reports its own readiness probe through `opengpu-agent health`
- carries that worker health snapshot through agent heartbeats so the control plane and dashboard can show backend readiness for each node

## What The Health Check Covers

`opengpu-agent health` now reports:

- whether the cached model exists
- whether `llama-cli` is available
- whether the `BLAS` device is available
- the current power source and battery state
- the runtime mode and collected notes

That probe is also attached to the signed heartbeat payload so the browser view can show worker readiness alongside the node state.
