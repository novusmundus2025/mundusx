# Apple Silicon Mac installation with MLX

Use this guide for an M-series Mac. The published macOS binaries target Apple
Silicon; this is not an Intel Mac installation guide. MLX is the native Apple
Silicon LLM runtime, not a separate operating system.

## 1. Open Terminal

Run under your normal macOS account. Keep the machine awake and allow enough
disk space for the selected model. Do not run the entire installer with `sudo`.

## 2. Install the commands

```bash
curl -fsSL https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
```

The script downloads the CLI, node agent, MundusX command, and agent server,
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

Follow the [shared wizard guide](contributor-setup.md). Select an offered
MLX-compatible managed model to use the native MLX path. If you choose an
existing supported model server instead, that server remains the runtime.
Source builds also support direct LM Studio with a loaded model; see [Inference connections](inference-connections.md). The published 0.2.19 bundle does not include this change.

Setup prepares an isolated Python virtual environment and MLX dependencies and
downloads the selected model. A Python virtual environment separates packages;
it is not an OS security sandbox.

**Does this require a `.pkg`?** The MundusX shell path does not require the
MundusX `.pkg`. If Python is already available, setup uses it. If Python is
missing, the current MLX bootstrap downloads and installs an official Python
package; macOS may request administrator authentication for that dependency.
Enter any OS password only in the local OS/terminal prompt, never in Chat.

## 4. Start and check readiness

```bash
opengpu start --background
opengpu status
opengpu doctor
opengpu model list
```

Complete onboarding when prompted. Check for `readyForJobs: yes`, a connected
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
Run `opengpu start --background` again when needed. Configuration and models
remain saved. The separate `mundusx connect` Chat workflow is not required here.

For images/video, see [media setup](install-media.md). On Mac, ComfyUI uses
PyTorch MPS/Metal, not MLX. Native setup is implemented, but the bundled models
must pass real verification on the specific Mac before media readiness is claimed.
