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
- `heartbeat.jsonl` - local heartbeat history, automatically reset every 30 minutes

## Current Behavior

The prototype agent:

- derives first-class scheduler roles from verified local capacity, including
  `chunk_analysis`, `reducer`, and `synthesizer`
- loads the existing provider config
- resolves the backend
- emits a registration payload
- sends registration and heartbeat updates to the control plane as signed device requests
- includes the machine hostname in the signed contributor identity
- polls the control plane for queued jobs
- continuously refills advertised parallel slots while existing jobs are running
- launches the local worker as a subprocess when requested
- sends a busy heartbeat before worker launch and a ready heartbeat after completion
- posts the worker result back to the control plane with output, error, duration, model/runtime, backend, worker ID, and node ID metadata
- converts subprocess launch failures into failed job completions so claimed jobs do not disappear silently
- includes the worker health snapshot in heartbeats so the control plane can surface backend readiness
- starts a pinned `llama-server` warm runtime when available, routes local inference through it, and falls back to batch `llama-cli` if the warm runtime is unavailable
- reports NVIDIA CUDA driver/device availability, device name, VRAM, and low-VRAM classification when the selected backend is `cuda`
- reports a scheduler capability profile in each heartbeat, including active model metadata, context budget, VRAM budget, load, roles, and backend/runtime tags
- treats the configured model directory as a local cache, not as something the control plane owns
- emits policy-aware heartbeats on a loop
- sends the power source, battery state, and policy allowance with each heartbeat
- writes the last heartbeat to disk
- treats paused or disconnected state as non-active
- exits and cools the persistent `llama-server` runtime when local config changes to paused or disconnected, so `opengpu exit`, `opengpu disconnect`, `Esc`, and `Ctrl-C` release GPU memory instead of leaving the model warm
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

- `opengpu start` auto-selects `cuda` when NVIDIA environment hints or `nvidia-smi` identify a CUDA-capable machine and keeps the foreground contribution session open.
- `vllm` is accepted only as an explicit backend preference for Linux/Ubuntu NVIDIA work. It is not auto-selected and does not start the llama.cpp warm runtime.
- `cudaDeviceAvailable: yes` means `nvidia-smi` found at least one NVIDIA GPU.
- `cudaDriverAvailable: yes` means the NVIDIA driver/runtime probe completed successfully.
- `cudaMemoryMb` reports the largest detected GPU memory total.
- `cudaLowVramProfile: yes` is selected at 4096 MB or below, including GTX 10-series 4 GB cards.
- missing drivers or CUDA runtime support produce an actionable health note and keep policy from allowing CUDA jobs.

The CUDA worker execution loop is still intentionally conservative. A low-VRAM node should advertise capability metadata and stay eligible only for modest CUDA work until model compatibility checks land.

The scheduler-facing node role profile is intentionally separate from the basic readiness gate. A node can be healthy enough to register but still publish no scheduler roles when policy, model compatibility, or worker health makes it ineligible. Ready CUDA, Vulkan, MLX, and vLLM nodes advertise roles such as `chat`, `coding`, and `batch`; nodes with enough VRAM, parallel capacity, vLLM runtime, or large MLX memory also advertise `reducer` so the control plane can route synthesis work away from small contributors.

The vLLM worker path is currently diagnostic/adapter-ready only. A node configured
with `backendPreference: vllm` reports vLLM readiness through `opengpu doctor`,
but the node agent keeps worker health unavailable until a real Linux vLLM
runtime adapter is installed.

### Building From Source Instead Of `install.ps1`

`install.ps1` is the only path that automatically downloads and pins the CUDA `llama.cpp` runtime (see [docs/install-strategy.md](install-strategy.md)). When the bundle includes `llama-server.exe`, the installer pins it as `llama_server` so `opengpu start` can keep the active GGUF model warm between jobs. If an older trusted-runtime file only pins `llama_cli`, the node agent will also accept a `llama-server` executable found beside that verified `llama-cli` path. A dev machine that instead runs:

```powershell
cargo build -p opengpu -p opengpu-node-agent --release
```

gets working `opengpu.exe`/`opengpu-node-agent.exe` binaries but no `llama-cli.exe`, so `opengpu-node-agent health` reports `llamaCliAvailable: no`, `blasDeviceAvailable`/`cudaDeviceAvailable: no` as applicable, and `policyAllowed: no`.

To fix that without running the full installer, either:

- run `scripts/windows-dev-llama-runtime.ps1`, which builds both binaries, downloads the CUDA `llama.cpp` release matching the machine's NVIDIA driver, and pins it in `trusted-runtime-paths.json` automatically (see `agents/node/README.md` for flags), or
- do it by hand:
  1. download the matching Windows CUDA build and `cudart` runtime zip from [ggml-org/llama.cpp releases](https://github.com/ggml-org/llama.cpp/releases) (match the CUDA version to `nvidia-smi`'s reported `CUDA Version`)
  2. extract both into `<OPENGPU_HOME>\runtimes\llama` (default `~\.opengpu\runtimes\llama`) so `llama-cli.exe` sits next to its CUDA DLLs
  3. compute its checksum with `Get-FileHash -Algorithm SHA256`
  4. write `<OPENGPU_HOME>\trusted-runtime-paths.json` pinning `llama_cli.path` (absolute) and `llama_cli.sha256`; also pin `llama_server.path` and `llama_server.sha256` when `llama-server.exe` is present. If only `llama_cli` is pinned, `llama-server.exe` can still be used when it sits in the same directory as the verified `llama-cli.exe`.
  5. re-run `opengpu-node-agent health` to confirm `llamaCliAvailable: yes` and `policyAllowed: yes`

Health reports `runtimeKind` as:

- `batch` when only `llama-cli` is available.
- `persistent-unavailable` when `llama-server` is pinned but no warm server is currently healthy.
- `persistent-warm` when a warm server is reachable through `OPENGPU_LLAMA_SERVER_URL`.

Set `OPENGPU_PERSISTENT_RUNTIME=off` to force batch mode. `opengpu start` owns the warm process in foreground mode and falls back to batch execution if startup or completion through the warm server fails. Disconnecting or exiting the node drops the warm runtime handle, stops `llama-server`, releases VRAM, and lets the GPU return to idle power.

## Job Lifecycle

When connected and policy-allowed, the run loop keeps heartbeats flowing while it checks for queued work:

1. Send registration and heartbeat payloads.
2. Poll `GET /v1/jobs/next?node_id=...`.
3. If no compatible job is available, keep heartbeating.
4. If a job is claimed, write and send a busy heartbeat.
5. Fill the node's advertised parallel slots and launch each claimed job with its worker profile.
6. While any job is running, poll for newly arrived work and immediately refill each free slot.
7. Post `completed` or `failed` to `POST /v1/jobs/complete` for every worker.
8. Include output, error, duration, model/runtime, backend, worker ID, and node ID in each completion report.
9. Write and send the next ready or paused heartbeat after no active or newly claimable work remains.

## UAT Rootless Code Verification

Linux contributors can opt into isolated Java verification after model generation. Compilation and execution happen on the contributor CPU, never in the control plane and never on the model GPU. The completion request carries the verification result inside the existing device-signed body.

The verifier is fail-closed. It is advertised as `sandbox_java` only when all of these are true:

- `OPENGPU_CODE_VERIFIER=podman` is set.
- Podman reports that it is running rootless.
- The configured Java image is already present locally; verification uses its immutable image ID and `--pull=never`.

Prepare a UAT contributor as its normal unprivileged service account:

```bash
podman pull docker.io/library/eclipse-temurin:21-jdk
podman info --format json
export OPENGPU_CODE_VERIFIER=podman
export OPENGPU_JAVA_VERIFIER_IMAGE=docker.io/library/eclipse-temurin:21-jdk
opengpu-agent run
```

The sandbox disables networking, uses a read-only root filesystem, drops all capabilities, enables `no-new-privileges`, bounds CPU, memory, PIDs and time, and mounts only a fresh temporary workspace. There is deliberately no direct host-execution or Docker-socket fallback. If the probe fails, jobs continue normally but their semantic status remains unverified.

For an adopted vLLM cluster, advertised slots use the runtime's configured
`max_num_seqs`, bounded by the KV cache's full-context sequence capacity. The
CLI reads only that numeric limit from local `/server_info` and refreshes it
when the cluster is re-verified; it does not persist the diagnostic response.

The contributed model's advertised output budget is derived from its capacity
tier (up to 16,384 tokens for server/synthesis nodes) and never exceeds the
context window reported by the serving runtime. It is not forced to the
2,048-token fallback used for small or unknown local models.

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
