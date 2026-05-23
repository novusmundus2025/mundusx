# Node Agent

The node agent is the background service that lives on a provider machine.

## Current Prototype Commands

- `opengpu-agent run` - start the local agent loop and emit heartbeats
- `opengpu-agent run --once` - print one registration + heartbeat payload and exit
- `opengpu-agent register` - print the registration payload
- `opengpu-agent heartbeat` - print one heartbeat payload
- `opengpu-agent launch-worker` - spawn the local worker process and print its result
- `opengpu-agent status` - show agent state and the last heartbeat saved locally
- `opengpu-agent health` - check whether the Mac worker runtime, device list, cached model, and policy state are ready
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
- sends registration and heartbeat updates to the control plane as signed device requests
- includes the machine hostname in the signed contributor identity
- polls the control plane for queued jobs
- claims one queued job at a time for the local node
- launches the local worker as a subprocess when requested
- sends a busy heartbeat before worker launch and a ready heartbeat after completion
- posts the worker result back to the control plane
- treats the configured model directory as a local cache, not as something the control plane owns
- emits policy-aware heartbeats on a loop
- sends the power source, battery state, and policy allowance with each heartbeat
- writes the last heartbeat to disk
- treats paused or disconnected state as non-active
- reports itself as paused to the control plane when the Mac policy says the node should not launch jobs
- currently speaks plain HTTP to the prototype control plane
- signs device requests with the existing Ed25519 device keypair

## Next Step

The next step is to connect the health check to live contribution limits and any future pause/resume policy.

## Local Development URL

For the current prototype, point the agent at:

```bash
opengpu config set control-plane-url http://127.0.0.1:8787
```

The prototype agent does not speak TLS yet, so `https://` URLs will be rejected with a helpful error.

For contributor-side model switching and cleanup rules, see [docs/model-lifecycle.md](/Users/DBATALL/Documents/aigrid/docs/model-lifecycle.md).

## Policy Controls

The health command now reports a policy result in addition to runtime health. If the model cache is missing, `llama-cli` is unavailable, the Mac worker is on battery with too high a contribution cap, or the contribution cap has not been set yet, the agent will skip job claims and report `policyAllowed: no`.
The agent also signs `register`, `heartbeat`, `jobs/next`, and `jobs/complete` requests so the control plane can verify the device by signature instead of a separate login flow. The signed payload includes the node ID, hostname, and device key material that identifies the contributor machine.
