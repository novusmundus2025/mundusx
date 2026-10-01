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
curl -fL --progress-bar --connect-timeout 20 https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
```

The script selects the architecture, downloads binaries/checksums, and prints
progress. On detected GB10/GX10 it also prepares the pinned vLLM container;
container layer downloads have their own progress display. This can take several
minutes. Do not prefix the whole command with `sudo`.

If you will reuse an existing engine, use this command **instead** to skip
automatic managed vLLM provisioning:

```bash
curl -fL --progress-bar --connect-timeout 20 https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash -s -- --connection direct --cluster-url http://127.0.0.1:8000
```

Replace the URL with your engine's actual endpoint. The bootstrap prints the
matching `opengpu install` command; it does not run the wizard automatically.

The default script installs first and leaves contributor choices to the wizard.
If a command is not found, open a new terminal or run:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 3. Prepare or discover the LLM runtime

For an existing vLLM, Ollama, LM Studio, llama.cpp, or supported OpenAI-compatible
server, keep it running and inspect discovery:

```bash
opengpu cluster scan
opengpu install --connection direct --cluster-url http://127.0.0.1:8000
```

Choose the exact model in setup. For a noninteractive selection, provide its ID:

```bash
opengpu install --connection direct --cluster-url http://127.0.0.1:8000 --cluster-model YOUR_MODEL_ID
```

The URL above is an example, not a requirement. Use the actual listening address.
A service listening on port 8000 is not automatically an LLM; ComfyUI or another
service on that port must not be selected as one. Detection validates the model
API. LM Studio must report the selected model as loaded. See
[Inference connections](inference-connections.md). MundusX does not provision
Sparkrun recipes, and PAIR currently supports validation only.

For managed GB10/GX10 vLLM, run `opengpu install --connection managed` instead
and choose an offered standalone model.
For general Linux x64, do not assume the installer also installed a local LLM
server; prepare a supported server before choosing that contribution path.

## 4. Configure, start, and verify

Finish the [shared setup choices](contributor-setup.md), then run:

```bash
opengpu --version
opengpu start --background
opengpu status
opengpu doctor
opengpu model list
```

This release reports `opengpu 0.2.23`. Complete onboarding if requested.
Confirm `readyForJobs: yes` and the expected
model/connection state. Inspect any reported failure reason before considering
the machine connected and ready. Keep an external model server running while
contributing it. Rerun setup if you change its model or endpoint.

## 5. Pause or disconnect

```bash
opengpu pause
opengpu resume
opengpu exit
```

After reboot, start the external engine and load its model first when using a
direct connection, then run `opengpu start --background`. Managed setup reloads
its saved active model. Use `opengpu logs` for logs and `opengpu doctor` for diagnostics.
For optional image/video setup, continue to [media setup](install-media.md).

## Installation progress

The initial script download shows curl progress. The installer then prints its
current step and download progress. Slow download, GPU checks, and Docker steps
print an elapsed-time update every 10 seconds; elapsed time is not a percentage
or an estimate of remaining time. A step failure prints its exit status.
Do not launch a second installer while one is still running.
