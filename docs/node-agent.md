# Node Agent

The node agent is the background service that lives on a provider machine.

## Current Prototype Commands

- `opengpu-agent run` - start the local agent loop and emit heartbeats
- `opengpu-agent run --once` - print one registration + heartbeat payload and exit
- `opengpu-agent register` - print the registration payload
- `opengpu-agent heartbeat` - print one heartbeat payload
- `opengpu-agent launch-worker` - spawn the local worker process and print its result
- `opengpu-agent status` - show agent state and the last heartbeat saved locally
- `opengpu-agent stop` - write a paused/offline state and exit

## What It Reuses

- the same `~/.opengpu/config.json` that the CLI writes
- the same `~/.opengpu/identity.json` device identity

## Local State Files

- `config.json` - user and provider settings
- `identity.json` - device key material and fingerprint
- `agent-state.json` - last heartbeat snapshot
- `heartbeat.jsonl` - append-only local heartbeat log

## Current Behavior

The prototype agent:

- loads the existing provider config
- resolves the backend
- emits a registration payload
- sends registration and heartbeat updates to the control plane
- polls the control plane for queued jobs
- claims one queued job at a time for the local node
- launches the local worker as a subprocess when requested
- sends a busy heartbeat before worker launch and a ready heartbeat after completion
- posts the worker result back to the control plane
- emits heartbeats on a loop
- writes the last heartbeat to disk
- treats paused or disconnected state as non-active
- currently speaks plain HTTP to the prototype control plane

## Next Step

The next step is to replace the simulated worker output with real `M` or `CUDA` execution.

## Local Development URL

For the current prototype, point the agent at:

```bash
opengpu config set control-plane-url http://127.0.0.1:8787
```

The prototype agent does not speak TLS yet, so `https://` URLs will be rejected with a helpful error.
