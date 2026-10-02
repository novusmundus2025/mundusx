# vLLM backend

## Muse context and chat latency (0.2.24)

Managed Muse-Glimmer runtimes now start with a 16,384-token context window.
Set `VLLM_MAX_MODEL_LEN` in `~/.opengpu/runtimes/vllm/runtime.conf`, or
`OPENGPU_VLLM_MAX_MODEL_LEN` in the agent environment, to choose a different
positive limit. The agent reads the selected model's actual `max_model_len`
from `/v1/models` when publishing scheduler capacity. Existing servers retain
their current limit until restarted; changing the advertisement alone does
not increase the model's serving capacity.

After upgrading an agent-managed runtime, run `opengpu disconnect`, then
`opengpu start`. For a separately managed container, change its
`--max-model-len` and restart it, then run
`opengpu cluster use http://127.0.0.1:8000 --max-jobs 4` and
`opengpu start --background` to discover its served capacity.

Ordinary Muse completion requests use the model template's `reasoning_strength:
low` setting. High reasoning previously delayed visible text even for greetings.
Native tool turns and structured-output requests retain their existing settings.

MundusX treats `vllm` as an explicit Linux NVIDIA backend. It is not selected by
`auto`, and it is not part of the Windows contributor path.

## Current contract

- Windows contributors keep using the signed `llama.cpp` CUDA runtime bundle.
- `opengpu start` with `backendPreference: auto` still detects Apple Silicon or
  CUDA only.
- `opengpu start --backend vllm` is accepted as an explicit preference, but the
  node agent advertises itself as ready only after its localhost vLLM endpoint
  passes the health probe.
- `opengpu doctor --json` includes a `vllm` diagnostic block with OS, Docker
  access, runtime configuration, NVIDIA driver visibility, endpoint health,
  and readiness details.

## Ubuntu ARM64 GB10/GX10 target

The first supported vLLM target is Ubuntu 24.04 on ARM64 GB10/GX10 systems with:

- NVIDIA GPU visible through `nvidia-smi`
- Docker Engine available to the installing user
- NVIDIA Container Toolkit available as `nvidia-ctk`
- GPU access working inside an NVIDIA CUDA container
- the pinned multi-architecture NVIDIA vLLM container
- a MundusX vLLM adapter that speaks the worker contract and returns
  output, error, model, runtime, backend, worker ID, and node ID metadata

Install the CLI, node agent, and pinned runtime configuration with:

```bash
bash install.sh
opengpu install
opengpu start
```

The installer detects GB10/GX10 automatically. Use `--with-vllm` to request the
same runtime explicitly on another supported Linux ARM64 NVIDIA host, or
`--without-vllm` to install only the MundusX CLI and node agent.

Use `--runtime-only` to configure the vLLM runtime around an existing MundusX
CLI installation. The installer binds the API to `127.0.0.1`, defaults
GPU memory utilization to `0.70`, limits concurrent sequences to four, and
stores its configuration under `~/.opengpu/runtimes/vllm/runtime.conf`.

When the contributor starts a connected vLLM node, the node agent starts the
pinned container for the active model, waits for `/health`, and routes jobs to
`/v1/chat/completions`. Pausing, disconnecting, or exiting the node stops the
container and releases the GPU. Nodes remain unavailable for scheduler claims
when the container, active model, or endpoint is not ready.

## Why this is separated from Windows

Windows stability is the release priority for the current contributor CLI. vLLM
is primarily a Linux serving stack, so Windows must not be forced to install
Python, CUDA developer tooling, or vLLM packages just to run the normal
`opengpu start` path.
