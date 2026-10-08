# GB10/GX10: start SparkRun before MundusX

**Important installation announcement:** for a GB10/GX10 contributor using an external vLLM runtime, prepare and start the model with SparkRun first, verify a local response, then connect MundusX with the direct connection option. This keeps runtime setup separate from contribution setup and avoids starting a second managed runtime.

These recipes target one Linux ARM64 GB10/GX10 with 128 GB unified memory. A larger NVIDIA machine is not automatically compatible: choose a container, quantization and recipe for its architecture. Start with one machine when the model fits; multiple machines are optional, not a requirement for coding.

## 1. Check prerequisites

Run on the contributing machine as your normal user:

```bash
uname -m
nvidia-smi
docker info
nvidia-ctk --version
```

Resolve driver, NVIDIA Container Toolkit and Docker access problems before continuing. Check that port 8000 is available and that another model is not already using the memory you intend to allocate. Keep the inference endpoint local unless you deliberately configure protected network access.

## 2. Install SparkRun

Follow the [upstream installation guide](https://sparkrun.dev/getting-started/quick-start/). If uv is not installed:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

Then run the setup wizard and check the installed command:

```bash
uvx sparkrun setup
sparkrun --help
```

For this guide, use the local single-machine setup. Multi-host SSH and networking setup is only needed when you intentionally distribute a model. Review the wizard's proposed changes before applying them.

## 3. Download and inspect one recipe

The Glimmer reference is based on the GX10 configuration that completed MundusX chat and coding requests. The Qwen3-Coder-Next NVFP4 reference was copied from the same machine's saved recipe; it has **not been rerun or benchmarked for this documentation update**. It is not a Qwen 30B recipe.

- [Glimmer 30B FP8 recipe](https://mundusx.github.io/mundusx/recipes/muse-glimmer-30b-fp8-gx10.yaml): general chat, coding and vision-capable model.
- [Qwen3-Coder-Next NVFP4 recipe](https://mundusx.github.io/mundusx/recipes/qwen3-coder-next-nvfp4-gx10.yaml): coding model; do not advertise vision from this recipe.

Both files pin the container digest present on GX10 during verification, bind to localhost, and use a single GPU. The changed localhost binding and fresh installation path have not been exercised by restarting the live machine. Model weights are not revision-pinned. Review model licensing and access requirements before downloading.

For Glimmer:

```bash
mkdir -p ~/mundusx-recipes
cd ~/mundusx-recipes
curl -fL -o muse-glimmer-30b-fp8-gx10.yaml https://mundusx.github.io/mundusx/recipes/muse-glimmer-30b-fp8-gx10.yaml
cat muse-glimmer-30b-fp8-gx10.yaml
sparkrun show ./muse-glimmer-30b-fp8-gx10.yaml
sparkrun run ./muse-glimmer-30b-fp8-gx10.yaml --solo
```

The first start downloads weights and prepares the runtime. Wait for readiness in its logs. For Qwen, download the Qwen file linked above and substitute its filename in the same commands. Run one recipe at a time on port 8000.

The reference settings reserve 80% GPU memory, allow 16 runtime sequences, and request 90,000 context tokens for Glimmer or 131,072 for Qwen. These are configuration ceilings, **not measured concurrency guarantees**. If startup runs out of memory, lower context length and concurrency in the recipe and verify again. MundusX contribution percentage is a separate scheduling setting; it does not reduce memory already allocated by SparkRun.

## 4. Verify the local model before contributing

In another terminal:

```bash
curl -f http://127.0.0.1:8000/health
curl -f http://127.0.0.1:8000/v1/models
curl -f http://127.0.0.1:8000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"muse-glimmer","messages":[{"role":"user","content":"Say hello in one sentence."}],"max_tokens":1024,"stream":false,"chat_template_kwargs":{"reasoning_strength":"low"}}'
```

For Qwen use the model ID returned by `/v1/models` (the reference alias is `qwen3-coder`) and omit Glimmer's `chat_template_kwargs`. Require a nonempty answer. A successful text test does not verify vision or tool calling; validate those separately before relying on their advertisement.

## 5. Install MundusX without provisioning another engine

```bash
curl -fL --progress-bar --connect-timeout 20 https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash -s -- --connection direct --cluster-url http://127.0.0.1:8000
export PATH="$HOME/.local/bin:$PATH"
opengpu cluster scan
opengpu install --connection direct --cluster-url http://127.0.0.1:8000 --cluster-model muse-glimmer
```

For Qwen replace `muse-glimmer` with `qwen3-coder`, or the exact ID returned by your server. Complete identity/onboarding, control-plane and contribution-cap choices. The control-plane allowed-model catalog still applies; an external runtime is not a way to bypass model policy. If setup rejects the model, ask the administrator to review its catalog entry and platform support.

Existing MundusX installations can run the direct-connection setup command without reinstalling binaries. Pause existing contribution before switching its model or endpoint.

## 6. Start and verify contribution

```bash
opengpu --version
opengpu start --background
opengpu status
opengpu doctor
```

Confirm `readyForJobs: yes`, backend `vllm`, the intended model and the actual available context. In the control plane check availability, contribution cap and advertised capabilities. A CUDA device is hardware; vLLM is the serving backend. Keep SparkRun running while contributing. MundusX adopts the endpoint and does not own its recipe or runtime lifecycle.

## 7. Restart, stop and troubleshoot

After reboot, verify the SparkRun runtime is serving before starting MundusX. Use `sparkrun status` to find the workload ID and `sparkrun logs JOB_ID` for startup errors. To stop deliberately, pause contribution with `opengpu pause`, then run `sparkrun stop JOB_ID` for the correct workload.

If local curl fails, resolve the runtime first. If local curl works but MundusX is unavailable, use `opengpu doctor` and check model policy, heartbeat and contribution settings. Measure local generation and MundusX assignment/response time separately; SparkRun does not guarantee faster responses or remove scheduler/network delays.

Upstream references: [quick start](https://sparkrun.dev/getting-started/quick-start/) and [recipe reference](https://github.com/spark-arena/sparkrun/blob/main/RECIPES.md). The saved Glimmer recipe uses the legacy `eugr-vllm` runtime identifier; consult upstream migration guidance before changing runtime versions.
