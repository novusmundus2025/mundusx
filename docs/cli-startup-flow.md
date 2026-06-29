# MundusX CLI Startup Flow

This document describes what the CLI does when a user starts using it for the first time and what each command is responsible for.

## First Run

The CLI owns the user-facing lifecycle. It manages local state and, when the user runs `opengpu start`, launches the installed node agent that connects the machine to the platform.

On first run, the CLI:

1. Creates a local config file if needed.
2. Generates a device ID.
3. Reuses an existing local secure identity if present, or creates one on first run.
4. Sets default values for:
   - connection state
   - pause state
   - backend preference
   - control-plane URL
   - contribution cap placeholder
5. Stores the config in the preferred config directory, or falls back to a local `.opengpu/config.json` file if needed.
6. If `OPENGPU_HOME` is set, that path wins over any repo-local fallback.

## Install

The platform bootstrapper installs the native `opengpu` binary only:

- macOS/Linux use `install.sh`.
- Windows uses `install.ps1`.
- The bootstrapper selects the release asset for the operating system and CPU architecture.
- The full contributor setup always happens inside `opengpu install`.

When the user runs:

```bash
opengpu install
```

the CLI runs the guided machine setup wizard:

1. Creates local config and secure device identity if needed.
2. Detects the machine profile, including OS, CPU architecture, Apple Silicon or CUDA backend, NVIDIA GPU name, and CUDA VRAM when available.
3. Asks which control plane to use:
   - public MundusX, which saves the hosted MundusX API endpoint
   - private / custom, which asks for a full `http://` or `https://` URL
   - blank control-plane input defaults to public MundusX
4. Asks how much of this machine's compute budget MundusX may use:
   - `20%` light
   - `30%` balanced
   - `50%` strong
5. Asks which model this node should run:
   - lighter safe catalog model
   - recommended safe catalog model
   - local GGUF / LM Studio model file
6. Filters model choices by the selected contribution cap and detected machine profile, then checks model fit before download or activation.
7. Saves the selected control-plane URL, contribution cap, and active model.
8. Prints the next step: `opengpu start`.

Scripted installs can pass flags instead of using the prompts:

```bash
opengpu install --public --cap-percent 30
opengpu install --private --control-plane-url http://127.0.0.1:8787 --cap-percent 30
```

## Login

When the user runs:

```bash
opengpu login
```

the CLI:

1. Saves an operator bearer token locally. On Windows, the token is protected with DPAPI outside `config.json`.
2. Keeps the machine ready for future control-plane calls.
3. Supports either an explicit `--token` value or an interactive prompt.

## Start

When the user runs:

```bash
opengpu start
```

the CLI:

1. Creates local config if needed.
2. Connects the machine locally when the secure device identity is available.
3. Clears the paused state only when the node can sign requests.
4. Detects the machine backend when possible:
   - Apple Silicon `aarch64` on macOS becomes `M`
   - NVIDIA CUDA machines become `CUDA` when CUDA environment hints or `nvidia-smi` are available
5. Optionally overrides that with `--m`.
6. If no contribution cap is saved yet, asks the user to choose the budget on interactive terminals.
   - `20%` light
   - `30%` balanced
   - `50%` strong
   - use the arrow keys and press Enter to confirm
   - press `Ctrl-C` to cancel the cap selector cleanly
   - scripted or non-interactive runs still print the `opengpu cap` hint instead of choosing silently
7. If no active model is saved yet, asks for a model choice, caches or imports it, and marks it active.
   - the starter model presets come from `apps/cli/config/official-models.json`
   - the starter presets point at public Hugging Face GGUF files compatible with the local Mac runtime, so no account is required for the default path
   - if the selected model is missing, the CLI downloads the public GGUF file and verifies the checksum when one is present in the catalog
8. Prints a startup summary with:
   - device ID
   - public key
   - public key fingerprint
   - identity readiness
   - platform
   - CPU core count
   - backend preference
   - detected backend when `backend preference` is `auto`
   - active model
   - model cache directory
   - contribution percent
   - connection state
   - pause state
   - config path
   - power source
   - battery percent
   - policy allowance
   - policy reason when the Mac should stay quiet
9. Prints the contributor onboarding checklist until it is marked complete:
   - device identity
   - hostname
   - backend
   - active model
   - contribution cap
   - policy state
   - credits link
   - dashboard link
10. Saves the updated config.
11. Prints how the contribution cap should be interpreted:
   - `M` means a memory-and-compute budget on Apple Silicon
   - `CUDA` means a routing budget for NVIDIA nodes, with low-VRAM cards kept to modest workloads
12. Prints whether policy currently allows the Mac to accept work, including the power source, battery state, and identity readiness.
13. Keeps the reused secure device identity attached to the local config when available.
14. Starts the installed `opengpu-node-agent` companion binary in the background when the secure device identity is ready.

For foreground diagnostics, run:

```bash
opengpu start --debug
```

Debug mode keeps the node agent attached to the terminal and prints the underlying agent logs. Direct `opengpu-node-agent run` remains an internal/developer entry point, not the normal contributor command.

## Operator Auth

If the control plane starts with `OPENGPU_OPERATOR_TOKEN`, the human-facing API routes and browser health page require a matching bearer token. The CLI stores that token locally with `opengpu login`, and clears it with `opengpu logout`. On Windows, the token is stored as a DPAPI-protected blob in the OpenGPU config directory, while `config.json` stays non-secret and safe to inspect.

## Connect

When the user runs:

```bash
opengpu connect
```

the CLI:

1. Marks the local config as connected.
2. Clears the paused state.
3. Reuses the active model cache if one already exists.
4. Prepares the machine to participate in routing once the control plane is online.
5. Reuses the active local model entry if it is already configured.
   - if the selected open model is missing, it is downloaded before being marked active

## Disconnect / Exit

When the user runs:

```bash
opengpu exit
```

`opengpu disconnect` remains available as the explicit advanced form.

the CLI:

1. Marks the local config as disconnected.
2. Pauses contribution.
3. Stops the background node agent started by `opengpu start` when one is recorded.
4. Leaves the identity and config in place for the next `start`.

## Status

When the user runs:

```bash
opengpu status
```

the CLI:

1. Reads the current local config.
2. Resolves the machine backend if `backend preference` is `auto`.
3. Treats the current machine as the active provider when connected and not paused.
4. Prints local provider state, active model, model cache directory, and the active backend decision.
5. Keeps sample node inventory out of the main status view for now.

## Onboarding

When onboarding has not been completed yet, `opengpu start` and `opengpu connect` print a dedicated checklist panel and a hint to run:

```bash
opengpu onboarding --complete
```

The onboarding command only marks the review step as complete; it does not change the device identity or control-plane state.

When the contribution cap has not been saved yet, `opengpu start` asks for it on an interactive terminal. `opengpu cap` remains the explicit command for changing or resetting it later.

Those deeper network behaviors will come later when the control plane and node agent are online.
