# Contributor setup choices and connection checklist

Use this after the [Windows](install-windows.md), [Linux](install-linux.md), or
[macOS/MLX](install-macos-mlx.md) binary installation. Screens depend on detected
hardware, existing servers, and saved configuration; the sequence below describes
the choices, not an exact transcript for every machine.

This guide follows release **0.2.22**. See [Inference connections](inference-connections.md)
for additional script options and external-engine requirements.

## 1. Run the wizard

```text
opengpu install
```

Windows normally opens this automatically in a fresh PowerShell window.
Check `opengpu --version` first if upgrading; this release reports `0.2.22`.

## 2. Choose one inference connection

For a fresh interactive setup, the first choice is:

```text
Choose one inference connection

> MundusX-managed runtime
    Download or reuse a standalone model
  Direct local engine
    Reuse Ollama, LM Studio, vLLM or llama.cpp
  NVIDIA PAIR cluster
    Validate endpoint only; contribution is not enabled yet

Up/Down: move | Space or Enter: select | Esc: cancel
```

This is a single choice, not a workload checkbox list. Explicit connection flags
skip this menu; older cluster flags can use the legacy discovery path.

- **Managed:** continue to cap/workload choices, then choose a standalone model.
- **Direct:** start your engine first, paste its actual endpoint when prompted,
  and choose the exact model. If only one is available, it is selected automatically.
  LM Studio must have the model loaded. Setup runs a short inference test before
  continuing. No model weights are downloaded by MundusX for this connection.
- **PAIR:** paste the endpoint shown by PAIR and choose a model to test. Setup
  verifies the route, explains that contribution is unavailable, and exits with
  code 2. It does not save a new contributor connection or continue to startup.

For example, a direct engine can be selected without the first menu:

```text
opengpu install --connection direct --cluster-url http://127.0.0.1:1234
```

The port is an example. Use the engine's own address; a PAIR proxy is not a direct
engine endpoint. See the [connection guide](inference-connections.md) for details.

## 3. Configure the network

Normal setup uses the public MundusX control plane. A new device identity is
created or the saved identity is reused. To configure a private control plane,
use `--private --control-plane-url URL` with your operator's URL and instructions.
The public contributor path does not require a GitHub token, browser pairing,
or pasting an account password. Development-agent Chat pairing is a separate flow.

## 4. Choose the contribution cap

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

## 5. Select workloads

The checkbox screen offers LLM, Qwen images, and Wan videos. Use arrows or Tab
to move, Space to toggle, and Enter to continue. Unavailable options remain
visible but disabled, with the reason shown. Image/video editing is unavailable.

Start with LLM if you only need a text contributor. Image/video selection also
requires runtime/model setup and a successful generation verification.

## 6. Prepare the chosen runtime and model

For a direct engine, the endpoint/model were selected earlier. Keep that engine
running. If the selected model becomes unavailable while the agent is running,
new job claims stop; they resume when the same model is ready again. Another
listed model is never silently substituted. Use setup or `opengpu cluster use`
to deliberately change the selection. An in-flight job can still fail if its
model is unloaded after the readiness check.

For managed inference, choose an offered model suitable for your hardware and budget. Setup
downloads missing files and prepares the managed runtime. Managed concurrency is
capacity-sized automatically, rather than always showing a manual job-count menu.
On Mac, the RAM ceilings are 1 below 32 GiB, 2 below 128 GiB, and 4 from 128 GiB.

## 7. Wait for setup and review its summary

Downloads display progress where available. Dependency installation, Docker
pulls/builds, and model loading may show textual logs instead of a single overall
percentage. Media generation uses elapsed time, not a reliable percentage bar.
Do not close the terminal during an active installation.

The summary distinguishes saved settings from verified readiness. If you selected
media, follow the [media guide](install-media.md) for setup/verification details.

## 8. Start contribution

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

## 9. Daily controls, reboot, and upgrading

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

Background startup and startup at sign-in are different. After reboot, a managed
runtime reloads the saved active model when started; it must warm up again. For
a direct engine, start that service and load the selected model first, then run
`opengpu start --background`. Model files and saved choices survive a reboot.

To upgrade an existing installation:

```text
opengpu update
opengpu --version
opengpu install
opengpu start --background
opengpu status
```

Check the setup summary and readiness again. Do not erase your identity or models
to upgrade. A running external API alone does not prove its model is warm.

## Startup progress

Release 0.2.22 shows runtime startup and warmup activity, including background mode.
See [CLI and agent progress](cli-progress.md) for download bars, byte totals, and
elapsed-time feedback across Windows, Linux, and macOS.
