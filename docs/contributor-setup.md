# Contributor setup choices and connection checklist

Use this after the [Windows](install-windows.md), [Linux](install-linux.md), or
[macOS/MLX](install-macos-mlx.md) binary installation. Screens depend on detected
hardware, existing servers, and saved configuration; the sequence below describes
the choices, not an exact transcript for every machine.

Release 0.2.20 and newer first offers managed models, a direct engine (including LM
Studio), or PAIR endpoint validation. See [Inference connections](inference-connections.md)
for the updated sequence and script options. These changes are not in the
older 0.2.19 bundle; PAIR contribution remains disabled.

## 1. Run the wizard

```text
opengpu install
```

Windows normally opens this automatically in a fresh PowerShell window.
Setup creates or reuses the device identity and scans running model servers.
It shows discovery progress before the configuration choices.

## 2. Choose the network

Choose the public MundusX control plane for normal contribution. Choose a private
control plane only when your operator has supplied its URL and instructions.
The public contributor path does not require a GitHub token, browser pairing,
or pasting an account password. Development-agent Chat pairing is a separate flow.

## 3. Choose the contribution cap

Select the percentage you want to contribute, between 1% and 80%. Saved values
may be preselected. Cancel before provisioning if you want to stop setup.

For media eligibility, the calculation is:

```text
contributed memory = detected physical memory × cap percentage / 100
image minimum = 32 GiB contributed memory
video minimum = 64 GiB contributed memory
```

| Physical memory | Cap | Contributed memory | Image selectable | Video selectable |
| --- | --- | --- | --- | --- |
| 16 GiB | 80% | 12.8 GiB | No | No |
| 32 GiB | 80% | 25.6 GiB | No | No |
| 48 GiB | 80% | 38.4 GiB | Yes | No |
| 64 GiB | 80% | 51.2 GiB | Yes | No |
| 96 GiB | 80% | 76.8 GiB | Yes | Yes |
| 128 GiB | 80% | 102.4 GiB | Yes | Yes |

These are illustrative exact GiB values. Actual detection uses bytes; a displayed,
rounded RAM size can differ near a threshold. Eligibility is not a guarantee that
the GPU, model, runtime, and available memory will pass real verification.

When contributing an existing server, MundusX does not own or enforce that
server's memory configuration. The setup displays automatic cluster contribution
and saves a local fallback cap; configure the external server's limits separately.

## 4. Select workloads

The checkbox screen offers LLM, Qwen images, and Wan videos. Use arrows or Tab
to move, Space to toggle, and Enter to continue. Unavailable options remain
visible but disabled, with the reason shown. Image/video editing is unavailable.

Start with LLM if you only need a text contributor. Image/video selection also
requires runtime/model setup and a successful generation verification.

## 5. Select an existing server or a managed model

An existing supported server can be offered through the contribution menu. If
selected, its loaded model is used and local LLM provisioning is skipped.
Concurrency may be requested for that server. Keep it running and avoid changing
its loaded model while contributing; pause and rerun setup to change configuration.

Otherwise, choose an offered model suitable for your hardware and budget. Setup
downloads missing files and prepares the managed runtime. Managed concurrency is
capacity-sized automatically, rather than always showing a manual job-count menu.
On Mac, the RAM ceilings are 1 below 32 GiB, 2 below 128 GiB, and 4 from 128 GiB.

## 6. Wait for setup and review its summary

Downloads display progress where available. Dependency installation, Docker
pulls/builds, and model loading may show textual logs instead of a single overall
percentage. Media generation uses elapsed time, not a reliable percentage bar.
Do not close the terminal during an active installation.

The summary distinguishes saved settings from verified readiness. If you selected
media, follow the [media guide](install-media.md) for setup/verification details.

## 7. Start contribution

```text
opengpu start --background
opengpu status
opengpu doctor
```

Review and complete onboarding when prompted. You are ready when the node is
connected, the intended model is active, the agent is running, and status reports
`readyForJobs: yes`. Media has its own readiness check. Idle/waiting for a job is
normal; a connected node is not guaranteed immediate work.

For a foreground troubleshooting session, disconnect the existing session with
`opengpu exit`, then run `opengpu start --debug`. Do not launch duplicate agents.

## 8. Daily controls and recovery

| Command | Purpose |
| --- | --- |
| `opengpu status` | Check connection and readiness |
| `opengpu doctor` | Inspect prerequisites/configuration |
| `opengpu logs` | Find local diagnostics |
| `opengpu pause` | Pause contribution |
| `opengpu resume` | Resume in the background |
| `opengpu cap` | Change the saved contribution cap |
| `opengpu model list` | Inspect local model choices |
| `opengpu exit` | Disconnect; retain saved identity/configuration/models |

Background startup and startup at sign-in are different. After reboot, check
status and run `opengpu start --background` if necessary. Rerunning the installer
downloads the current production assets; exit the existing node first, then
rerun setup and verify readiness. Do not erase your device identity to upgrade.

Release note: the published bundle tag is 0.2.19, but its CLI package metadata
still prints 0.2.16 with `--version`. Do not use that string alone to determine
whether the current release assets were downloaded.
