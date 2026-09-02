# OpenGPU Getting Started

This guide installs a MundusX OpenGPU contributor node. It is not the coding
Harness installer and it does not require GitHub authentication, a Git token,
Cargo, Rust, or browser pairing.

## Choose the instructions for the machine being installed

| Machine | Terminal | Installer | Runtime behavior |
| --- | --- | --- | --- |
| ASUS Ascent GX10 / NVIDIA GB10 | Linux shell | `install.sh` | Installs the Linux ARM64 node and pinned NVIDIA vLLM runtime |
| Linux x86_64 | Linux shell | `install.sh` | Installs the x86_64 node; use an existing supported local model server |
| Apple Silicon Mac | Terminal or Finder | `install.sh` or `.pkg` | Installs the Apple Silicon node; guided setup prepares MLX |
| Windows x86_64 | PowerShell | `install.ps1` | Installs the Windows node, tray application, and detected GPU runtime |

The public release channel is:

<https://github.com/mundusx/releases/releases/tag/opengpu-prod>

## ASUS Ascent GX10 / NVIDIA GB10

Run these commands **on the GX10**, not in Windows PowerShell. If working from
another computer, connect to the GX10 first:

```bash
ssh <username>@<gx10-address>
```

Install the OpenGPU binaries, node agent, and pinned vLLM runtime:

```bash
curl -fsSL https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
```

The installer displays progress for each binary, checksum, NVIDIA validation
image, and vLLM image layer. The container and model downloads can take several
minutes depending on the connection.

Open the guided contributor setup:

```bash
opengpu install
```

The wizard asks the contributor to choose:

1. Public MundusX or a private control plane.
2. The contribution cap, from 1% through 80%.
3. The maximum number of concurrent jobs.
4. An existing local model cluster, when one is detected, or a managed model.
5. A model compatible with the machine and selected contribution cap.

Start contribution after reviewing the choices:

```bash
opengpu start --background
```

The GX10 operating system normally includes Docker and NVIDIA Container Toolkit.
If installation stops at a prerequisite check, verify them with:

```bash
nvidia-smi
docker --version
nvidia-ctk --version
docker info
```

If only `docker info` reports a permission error:

```bash
sudo usermod -aG docker "$USER"
newgrp docker
docker info
```

Then run the installer again. It is safe to repeat.

## Linux x86_64

Run in a Linux shell:

```bash
curl -fsSL https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
opengpu install
opengpu start --background
```

The general Linux x86_64 package installs the CLI and node agent but does not
install the GX10-specific vLLM container. A Linux x86_64 contributor should run
a supported local server such as vLLM, Ollama, llama.cpp, or an
OpenAI-compatible server. `opengpu install` detects supported localhost servers
and offers the loaded model for contribution.

Inspect detected servers before setup when needed:

```bash
opengpu cluster scan
```

## Apple Silicon macOS

Open Terminal on an M-series Mac and run:

```bash
curl -fsSL https://github.com/mundusx/releases/releases/download/opengpu-prod/install.sh | bash
opengpu install
opengpu start --background
```

For a normal macOS package installation, download
[`MundusX-OpenGPU-Apple-Silicon.pkg`](https://github.com/mundusx/releases/releases/download/opengpu-prod/MundusX-OpenGPU-Apple-Silicon.pkg)
and open it in Finder. The shell script and package install the same `opengpu`
and `opengpu-node-agent` commands. Until MundusX configures Apple Developer ID
signing and notarization, macOS may require an explicit Open action for the
package; the checksum remains published beside it.

The installer selects the Apple Silicon binary. The guided setup prepares the
local MLX runtime and shows model-download progress. Intel Macs are not included
in the current public channel.

## Windows x86_64

Do not run the Linux `curl ... | bash` command in PowerShell. In Windows
PowerShell, `curl` can be an alias for `Invoke-WebRequest`, and Bash scripts are
not Windows installers.

Download and run the reviewed PowerShell installer as one command:

```powershell
$script = Join-Path $env:TEMP "mundusx-install.ps1"; Invoke-WebRequest -Uri "https://github.com/mundusx/releases/releases/download/opengpu-prod/install.ps1" -OutFile $script; powershell -NoProfile -ExecutionPolicy Bypass -File $script
```

The installer downloads the CLI, node agent, tray application, checksums, and
the detected Windows GPU runtime with visible progress. When it completes, open
a new PowerShell window and run `opengpu install` so the contributor can choose
the cap, concurrency, and model. The transparent script is the recommended
Windows path until the graphical installer is Authenticode-signed.

After the wizard completes, start contribution from PowerShell:

```powershell
& "$env:USERPROFILE\.opengpu\bin\opengpu.exe" start --background
```

Use the full path above if `opengpu` is not yet available on `PATH`.

## Verify the contributor node

Linux and macOS:

```bash
opengpu status
opengpu doctor
opengpu model list
```

Windows PowerShell:

```powershell
$opengpu = "$env:USERPROFILE\.opengpu\bin\opengpu.exe"
& $opengpu status
& $opengpu doctor
& $opengpu model list
```

`readyForJobs: yes` means the node has a secure device identity, an active
model, an allowed contribution policy, and a running node agent. When it says
`no`, `status` or `doctor` prints the reason.

## Everyday controls

```bash
opengpu status
opengpu pause
opengpu resume
opengpu cap
opengpu model use
opengpu exit
```

- `pause` stops accepting new jobs without deleting local configuration.
- `resume` returns the configured node to background contribution.
- `cap` changes how much of the machine may be used.
- `model use` changes the active model.
- `exit` disconnects the node and releases its managed runtime.

Configuration, device identity, runtime settings, and managed models remain in
the user's OpenGPU directory. On Linux and macOS this is normally `~/.opengpu`;
on Windows it is normally `%USERPROFILE%\.opengpu`.

After a machine reboot, the configuration remains. If the node is not already
running, start it again with:

```bash
opengpu start --background
```

## Common mistakes

### PowerShell rejects `-fsSL`

The Linux command was entered in Windows PowerShell. Use the Windows installer,
or SSH into the Linux GX10 and run the command there.

### A copied command contains `[text](url)`

That is Markdown link notation, not part of the URL. Copy commands from fenced
code blocks exactly as shown in this guide.

### `opengpu` is not found after Linux or macOS installation

Open a new terminal. If it is still missing, add the user binary directory:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Stop safely

```bash
opengpu exit
```

This keeps the identity, configuration, and downloaded models for the next
start.
