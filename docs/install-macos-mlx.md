# Apple Silicon Mac installation with MLX

Use this guide for an M-series Mac. The published macOS binaries target Apple
Silicon; this is not an Intel Mac installation guide. MLX is the native Apple
Silicon LLM runtime, not a separate operating system.

## 1. Open Terminal

Run under your normal macOS account. Keep the machine awake and allow enough
disk space for the selected model. Do not run the entire installer with `sudo`.

## 2. Install the commands

```bash
curl -fL --progress-bar --connect-timeout 20 https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
```

The script downloads the OpenGPU CLI and node agent,
and displays download/checksum progress. The default is install-only; it prints
the next commands rather than immediately starting contribution.

If `opengpu` is not found, open a new Terminal window. For the current shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 3. Choose your contribution and model

```bash
opengpu install
```

Follow the [shared wizard guide](contributor-setup.md). First choose
**MundusX-managed runtime**, then select workloads and an offered MLX-compatible
model to use the native MLX path. The explicit command is
`opengpu install --connection managed`.

To reuse Ollama or LM Studio instead, start that engine first. Load an LLM in
LM Studio and enable its API server, then run (using your actual endpoint):

```bash
opengpu install --connection direct --cluster-url http://127.0.0.1:1234
```

Select the exact model and wait for its verification request. This uses the
external engine and skips MundusX MLX/model provisioning. See
[Inference connections](inference-connections.md) for details. PAIR is currently
validation-only and cannot start contribution.

For managed MLX, setup prepares an isolated Python virtual environment and dependencies and
downloads the selected model. A Python virtual environment separates packages;
it is not an OS security sandbox.

**Does this require a `.pkg`?** The MundusX shell path does not require the
MundusX `.pkg`. If Python is already available, setup uses it. If Python is
missing, the current MLX bootstrap downloads and installs an official Python
package; macOS may request administrator authentication for that dependency.
Enter any OS password only in the local OS/terminal prompt, never in Chat.

## 4. Start and check readiness

```bash
opengpu --version
opengpu start --background
opengpu status
opengpu doctor
opengpu model list
```

This release reports `opengpu 0.2.23`. Complete onboarding when prompted.
Check for `readyForJobs: yes`, a connected
node, and the intended active model. MLX attempts to launch a persistent server
and runs a small inference warmup. Startup/model loading can take time. If the
persistent path fails, fallback execution can be used; do not assume every
successful connection is a warm persistent MLX server. Inspect diagnostics.

## 5. Job limits and stopping

The Mac memory ceiling is 1 concurrent job below 32 GiB RAM, 2 from 32 GiB up
to below 128 GiB, and 4 at 128 GiB or above. Actual model capacity can be lower.

```bash
opengpu pause
opengpu resume
opengpu exit
```

Background execution does not itself promise automatic startup after reboot.
For direct connections, start the external engine and load its selected model
before `opengpu start --background`. Managed MLX reloads and warms its saved
active model on startup. Configuration and models
remain saved. The separate `mundusx connect` Chat workflow is not required here.

For images/video, see [media setup](install-media.md). On Mac, ComfyUI uses
PyTorch MPS/Metal, not MLX. Native setup is implemented, but the bundled models
must pass real verification on the specific Mac before media readiness is claimed.

## Installation progress

The initial script download shows curl progress. The installer then prints its
current step and download progress. Slow download, GPU checks, and Docker steps
print an elapsed-time update every 10 seconds; elapsed time is not a percentage
or an estimate of remaining time. A step failure prints its exit status.
Do not launch a second installer while one is still running.

## Managed MLX context and streaming

The contributor reads the selected model's cached `config.json` and complete
weight files. For supported standard-attention architectures, including
Qwen3-Coder 30B, it advertises the smaller of the model context window and an
estimated memory budget. The estimate reserves memory for runtime/prefill
work and divides full-precision KV-cache capacity across all advertised job
slots. Four-bit weights do not imply a four-bit KV cache. Unsupported or
missing metadata retains the conservative fallback; no network lookup occurs
on a heartbeat. This estimate needs hardware verification for long prompts.

To request a specific ceiling before starting the node:

```bash
export OPENGPU_MODEL_CONTEXT_TOKENS=81920
opengpu start --no-contribute-cluster
```

The override is still bounded by model metadata and the contribution memory
budget. Lower `--max-jobs` if you need more context per concurrent request.
It applies to managed MLX; contributed engines keep their reported limits.
The node does not truncate input to make a request fit.

Managed MLX startup allows 900 seconds by default. To change that deadline:

```bash
export OPENGPU_MLX_START_TIMEOUT_SECONDS=1200
```

A complete one-token SSE generation verifies streaming before admission.
A separate harmless function-call probe verifies native tool support; failed
tool probes do not prevent ordinary chat. A failed warm-up leaves streaming
unavailable and identifies the runtime log. Owned MLX servers disable retained
prompt caches with `--prompt-cache-size 0`, so old long-context conversations
do not silently exceed the active-slot memory estimate. This may reduce
repeat-prompt performance; active generation KV caches are still used.

After updating and restarting, check `opengpu status --json` for the selected
model's `context_tokens`, `streaming_supported`, and `supports_tools` rather
than assuming every installed model advertises those capabilities.
