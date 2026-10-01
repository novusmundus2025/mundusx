# Linux contributor installation

The public channel includes Linux x86_64 and ARM64 binaries. Run these commands
in a Linux terminal on the contributing machine, under your normal user account.

## 1. Identify the machine

```bash
uname -m
```

`x86_64` selects the x64 release; `aarch64`/`arm64` selects ARM64. Automatic
managed vLLM provisioning is specifically for detected ARM64 GB10/GX10 systems.
Other Linux systems can contribute an existing supported local LLM server.

On a GB10/GX10, check the existing NVIDIA/container installation:

```bash
nvidia-smi
docker --version
nvidia-ctk --version
docker info
```

Resolve missing GPU/container prerequisites or Docker access before provisioning
vLLM. Use the machine administrator's approved setup. Docker access grants broad
host privileges; do not add users to the Docker group casually.

## 2. Install the commands

```bash
curl -fsSL https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
```

The script selects the architecture, downloads binaries/checksums, and prints
progress. On detected GB10/GX10 it also prepares the pinned vLLM container;
container layer downloads have their own progress display. This can take several
minutes. Do not prefix the whole command with `sudo`.

The default script installs first and leaves contributor choices to the wizard.
If a command is not found, open a new terminal or run:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 3. Prepare or discover the LLM runtime

For an existing vLLM, Ollama, llama.cpp, or other supported OpenAI-compatible
server, keep it running and inspect discovery:

```bash
opengpu cluster scan
opengpu install
```

Choose it in setup if you intend to contribute that server's loaded model.
If it is not discovered, specify its actual model API endpoint:

```bash
opengpu install --cluster-url http://127.0.0.1:8000
```

The URL above is an example, not a requirement. Use the actual listening address.
A service listening on port 8000 is not automatically an LLM; ComfyUI or another
service on that port must not be selected as one. Detection validates the model
API. Source builds also support direct LM Studio with a loaded model; see [Inference connections](inference-connections.md). MundusX does not provision Sparkrun recipes.

For managed GB10/GX10 vLLM, follow the offered managed model path instead.
For general Linux x64, do not assume the installer also installed a local LLM
server; prepare a supported server before choosing that contribution path.

## 4. Configure, start, and verify

Finish the [shared setup choices](contributor-setup.md), then run:

```bash
opengpu start --background
opengpu status
opengpu doctor
opengpu model list
```

Complete onboarding if requested. Confirm `readyForJobs: yes` and the expected
model/connection state. Inspect any reported failure reason before considering
the machine connected and ready. Keep an external model server running while
contributing it. Rerun setup if you change its model or endpoint.

## 5. Pause or disconnect

```bash
opengpu pause
opengpu resume
opengpu exit
```

After reboot, start again with `opengpu start --background` if the node is not
running. Use `opengpu logs` for log locations and `opengpu doctor` for diagnostics.
For optional image/video setup, continue to [media setup](install-media.md).
