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

- performs deterministic local prompt analysis and compute
- returns a completed response
- chooses a concrete backend when `auto` is passed
- on macOS `M`, prefers a native Swift/Metal compute path and falls back to deterministic compute if Metal is unavailable
- is normally launched by the node agent, not run directly by users

## Next Step

Replace the remaining deterministic fallback with a real `CUDA` execution path and a model-aware Apple Silicon inference path.
