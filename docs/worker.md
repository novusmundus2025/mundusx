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

- performs real local inference on macOS `M` by calling `llama.cpp` against the cached GGUF model
- runs in single-turn batch mode via `llama-cli --device BLAS`
- returns the generated text from the local model
- chooses the Mac `M` path when `auto` is passed on Apple Silicon
- is normally launched by the node agent, not run directly by users

## Next Step

Add health checks and policy controls around the Mac model runner.
