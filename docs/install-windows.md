# Windows contributor installation

This guide installs an OpenGPU compute contributor on Windows x86_64. It uses
the public release in `mundusx/releases`. No Rust, GitHub token, Chat pairing,
or development workspace is required for this contributor-only path.

## 1. Prepare Windows

Use an ordinary PowerShell window under the account that will contribute.
Have a working GPU driver, internet access, and enough disk space for the
runtime and chosen models. NVIDIA machines use the CUDA runtime by default;
other machines receive Vulkan. Runtime download alone does not prove GPU support.

Do not paste the Linux/macOS `curl ... | bash` command into PowerShell.

## 2. Download and run the installer

Paste this entire line into PowerShell:

```powershell
$installerPath = Join-Path $env:TEMP "mundusx-install.ps1"; Invoke-WebRequest -Uri "https://github.com/mundusx/releases/releases/download/opengpu-prod/install.ps1" -OutFile $installerPath; powershell -NoProfile -ExecutionPolicy Bypass -File $installerPath -SetupMode contributor -SkipTrayAutoStart
```

This explicitly selects compute contribution and disables tray startup at sign-in.
Without `-SetupMode contributor`, the installer offers developer, contributor,
or both. Without `-SkipTrayAutoStart`, it configures tray auto-start.

**Already using a local engine?** To skip the managed CUDA/Vulkan download, use
this command instead of the line above (replace the example URL with the actual
engine endpoint):

```powershell
$installerPath = Join-Path $env:TEMP "mundusx-install.ps1"; Invoke-WebRequest -Uri "https://github.com/mundusx/releases/releases/download/opengpu-prod/install.ps1" -OutFile $installerPath; powershell -NoProfile -ExecutionPolicy Bypass -File $installerPath -SetupMode contributor -SkipTrayAutoStart -Connection direct -ClusterUrl http://127.0.0.1:1234
```

Start Ollama, LM Studio, vLLM, or llama.cpp before setup. LM Studio must have an
LLM loaded and its API server enabled. The normal installer line downloads the
managed GPU runtime before the CLI wizard, even if you later choose a direct
engine there; use `-Connection direct` to avoid that download.

Expect these six installer phases, with download progress where available:

1. Downloading OpenGPU CLI.
2. Verifying the signed release.
3. Downloading the GPU runtime, or skipping it for an external connection.
4. Downloading the OpenGPU node agent.
5. Downloading the Windows tray application.
6. Installing verified components.

The default binary directory is `%USERPROFILE%\.opengpu\bin`. A fresh
PowerShell window opens with `opengpu install` running. If it does not, run:

```powershell
& "$env:USERPROFILE\.opengpu\bin\opengpu.exe" install
```

## 3. Complete the setup wizard

Follow the [shared setup choices](contributor-setup.md): choose one connection
(managed model, direct engine, or PAIR validation), then complete the contribution
cap, workloads, and any managed model/runtime preparation. Direct setup verifies
the exact selected model. PAIR currently validates only and cannot contribute.
Review the setup summary. Wait for model downloads to finish.
Public contribution does not ask you to paste a GitHub token or account password.

## 4. Start and verify

In PowerShell:

```powershell
$opengpu = "$env:USERPROFILE\.opengpu\bin\opengpu.exe"
& $opengpu start --background
& $opengpu status
& $opengpu doctor
```

`& $opengpu --version` should report `opengpu 0.2.21` for this release.

Complete contributor onboarding if prompted. Look for `readyForJobs: yes` and
check the reported connection/model state. A completed download or saved setup
is not the same as a connected, ready contributor. Follow any reason shown by
`status` or `doctor` before proceeding.

## 5. Everyday commands

```powershell
& $opengpu pause
& $opengpu resume
& $opengpu cap
& $opengpu model list
& $opengpu exit
```

The `$opengpu` variable lasts only in the current PowerShell window; set it again
in a new window. After reboot, start an external engine and load its selected
model first if using a direct connection, then run `start --background`.
For installation errors, inspect `%USERPROFILE%\.opengpu\logs\installer.log`.
For node diagnostics, run `& $opengpu logs`. Redact secrets before sharing logs.

For optional images/video, see [media setup](install-media.md). Managed Windows
ComfyUI currently requires NVIDIA CUDA and Python 3.12 or 3.13; a successful
Vulkan LLM installation does not imply managed media support.
