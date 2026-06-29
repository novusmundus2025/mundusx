# Node Agent

The node agent is the background service that lives on a provider machine.

## Current Prototype Commands

- `opengpu-agent run` - start the local agent loop and emit heartbeats
- `opengpu-agent run --once` - print one registration + heartbeat payload and exit
- `opengpu-agent register` - print the registration payload
- `opengpu-agent heartbeat` - print one heartbeat payload
- `opengpu-agent launch-worker` - spawn the local worker process and print its result
- `opengpu-agent status` - show agent state and the last heartbeat saved locally
- `opengpu-agent health` - check whether the local worker runtime, device list, cached model, power state, CUDA diagnostics, and policy state are ready
- `opengpu-agent stop` - write a paused/offline state and exit

## What It Reuses

- the same `~/.opengpu/config.json` that the CLI writes
- the same `~/.opengpu/identity.json` device identity

## Local State Files

- `config.json` - user and provider settings
- `identity.json` - public device metadata, fingerprint, and encrypted device identity blob
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
- posts the worker result back to the control plane with output, error, duration, model/runtime, backend, worker ID, and node ID metadata
- converts subprocess launch failures into failed job completions so claimed jobs do not disappear silently
- includes the worker health snapshot in heartbeats so the control plane can surface backend readiness
- reports NVIDIA CUDA driver/device availability, device name, VRAM, and low-VRAM classification when the selected backend is `cuda`
- treats the configured model directory as a local cache, not as something the control plane owns
- emits policy-aware heartbeats on a loop
- sends the power source, battery state, and policy allowance with each heartbeat
- writes the last heartbeat to disk
- treats paused or disconnected state as non-active
- reports itself as paused to the control plane when the Mac policy says the node should not launch jobs
- currently speaks plain HTTP to the prototype control plane
- signs device requests with the existing device keypair through the local OS signer

## Next Step

The health check is already connected to live contribution limits and pause policy. The worker backend health snapshot is now carried through to the control plane and browser views. CUDA nodes now use `nvidia-smi` to report the NVIDIA device name, total VRAM, and whether the machine should use the low-VRAM profile.

## Windows NVIDIA Bring-Up

For the first Windows contributor path, use a modest NVIDIA machine as a real capability probe rather than assuming it can run every CUDA workload.

```bash
opengpu start
opengpu-agent health
opengpu-agent register --json
opengpu-agent heartbeat --json
```

Expected health behavior:

- `opengpu start` auto-selects `cuda` when NVIDIA environment hints or `nvidia-smi` identify a CUDA-capable machine.
- `cudaDeviceAvailable: yes` means `nvidia-smi` found at least one NVIDIA GPU.
- `cudaDriverAvailable: yes` means the NVIDIA driver/runtime probe completed successfully.
- `cudaMemoryMb` reports the largest detected GPU memory total.
- `cudaLowVramProfile: yes` is selected at 4096 MB or below, including GTX 10-series 4 GB cards.
- missing drivers or CUDA runtime support produce an actionable health note and keep policy from allowing CUDA jobs.

The CUDA worker execution loop is still intentionally conservative. A low-VRAM node should advertise capability metadata and stay eligible only for modest CUDA work until model compatibility checks land.

## Job Lifecycle

When connected and policy-allowed, the run loop keeps heartbeats flowing while it checks for queued work:

1. Send registration and heartbeat payloads.
2. Poll `GET /v1/jobs/next?node_id=...`.
3. If no compatible job is available, keep heartbeating.
4. If a job is claimed, write and send a busy heartbeat.
5. Launch the local worker with the claimed job profile.
6. Post `completed` or `failed` to `POST /v1/jobs/complete`.
7. Include output, error, duration, model/runtime, backend, worker ID, and node ID in the completion report.
8. Write and send the next ready or paused heartbeat.

## Local Development URL

For the current prototype, point the agent at:

```bash
opengpu config set control-plane-url http://127.0.0.1:8787
```

The prototype agent does not speak TLS yet, so `https://` URLs will be rejected with a helpful error.

## No-Auth Local Smoke

Use the repository smoke script when you need to prove the local/UAT no-auth loop across the CLI, node agent, and sibling control-plane checkout:

```bash
./scripts/no-auth-e2e-smoke.sh
```

The script starts the control plane with `MUNDUSX_AUTH_DISABLED=true`, confirms `/health` reports `operator_auth_enforced=false`, seeds an isolated temporary `OPENGPU_HOME`, registers and heartbeats a signed node, submits a job without an operator token, lets the node agent claim and complete it, and polls the final result with `opengpu jobs wait`.

By default it expects the sibling control-plane repo at `../control-plane` and uses local `http://127.0.0.1:8787`. Override `CONTROL_PLANE_REPO`, `CONTROL_PLANE_URL`, `CONTROL_PLANE_HOST`, `CONTROL_PLANE_PORT`, or `OPENGPU_SMOKE_HOME` for UAT/local variants.

For contributor-side model switching and cleanup rules, see [docs/model-lifecycle.md](/Users/DBATALL/Documents/mundusx/docs/model-lifecycle.md).

## Policy Controls

The health command now reports a policy result in addition to runtime health. If the model cache is missing for the Mac path, `llama-cli` is unavailable, CUDA diagnostics fail for the CUDA path, the Mac worker is on battery with too high a contribution cap, or the contribution cap has not been set yet, the agent will skip job claims and report `policyAllowed: no`.
The agent also signs `register`, `heartbeat`, `jobs/next`, and `jobs/complete` requests so the control plane can verify the device by signature instead of a separate login flow. The signed payload includes the node ID, hostname, `identityTrustPath`, and the public device identity that identifies the contributor machine. On macOS, the private key is kept encrypted-at-rest inside the local identity record, and the decryption secret uses the macOS Keychain when available with a local fallback when it is not. On Linux, the same encrypted-at-rest identity record is used with Secret Service through `secret-tool` when available, and `identityTrustPath` reports either `secret-service://mundusx/device-identity` or the visible local encrypted fallback.
