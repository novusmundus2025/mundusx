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

Expect these six installer phases, with download progress where available:

1. Downloading OpenGPU CLI.
2. Verifying the signed release.
3. Downloading the GPU runtime; CUDA can be a large download.
4. Downloading the OpenGPU node agent.
5. Downloading the Windows tray application.
6. Installing verified components.

The default binary directory is `%USERPROFILE%\.opengpu\bin`. A fresh
PowerShell window opens with `opengpu install` running. If it does not, run:

```powershell
& "$env:USERPROFILE\.opengpu\bin\opengpu.exe" install
```

## 3. Complete the setup wizard

Follow the [shared setup choices](contributor-setup.md): public control plane,
contribution cap, workloads, existing server or managed model, and any runtime
preparation. Review the setup summary. Wait for model downloads to finish.
Public contribution does not ask you to paste a GitHub token or account password.

## 4. Start and verify

In PowerShell:

```powershell
$opengpu = "$env:USERPROFILE\.opengpu\bin\opengpu.exe"
& $opengpu start --background
& $opengpu status
& $opengpu doctor
```

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
in a new window. After reboot, run `start --background` when you want to contribute.
For installation errors, inspect `%USERPROFILE%\.opengpu\logs\installer.log`.
For node diagnostics, run `& $opengpu logs`. Redact secrets before sharing logs.

For optional images/video, see [media setup](install-media.md). Managed Windows
ComfyUI currently requires NVIDIA CUDA and Python 3.12 or 3.13; a successful
Vulkan LLM installation does not imply managed media support.
