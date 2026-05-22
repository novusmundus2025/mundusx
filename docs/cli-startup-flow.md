# OpenGPU CLI Startup Flow

This document describes what the CLI does when a user starts using it for the first time and what each command is responsible for.

## First Run

The CLI does not start a long-running service by itself. Instead, it manages local state and prepares the machine to connect to the platform.

When the user runs:

```bash
opengpu init
```

the CLI:

1. Creates a local config file.
2. Generates a device ID.
3. Reuses an existing local keypair if present, or generates one on first run.
4. Sets default values for:
   - connection state
   - pause state
   - backend preference
   - control-plane URL
   - contribution cap placeholder
5. Stores the config in the preferred config directory, or falls back to a local `.opengpu/config.json` file if needed.
6. If `OPENGPU_HOME` is set, that path wins over any repo-local fallback.

## Login

When the user runs:

```bash
opengpu login
```

the CLI:

1. Saves an auth token locally.
2. Optionally stores a profile name.
3. Keeps the machine ready for future control-plane calls.

## Start

When the user runs:

```bash
opengpu start
```

the CLI:

1. Creates local config if needed.
2. Connects the machine locally.
3. Clears the paused state.
4. Detects the machine backend when possible:
   - Apple Silicon `aarch64` on macOS becomes `M`
   - CUDA hints in the environment become `CUDA`
5. Optionally overrides that with `--m` or `--cuda`.
6. If no contribution cap is saved yet, shows a compact retro vertical selector for:
   - `20%` light
   - `30%` balanced
   - `50%` strong
   - `75%` aggressive
   - `90%` max
   - use the arrow keys and press Enter to confirm
   - press `Ctrl-C` to abort the active `opengpu start` session cleanly and roll back to disconnected/paused
7. If no active model is saved yet, caches a local model entry and marks it active.
   - the starter model presets come from `apps/cli/config/official-models.json`
   - the current CLI still does not fetch real model weights from a remote registry
8. Prints a startup summary with:
   - device ID
   - public key
   - public key fingerprint
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
9. Saves the updated config.
10. Prints how the contribution cap should be interpreted:
   - `M` means a memory-and-compute budget on Apple Silicon
   - `CUDA` means a GPU-utilization budget on NVIDIA nodes
11. Keeps the reused device identity attached to the local config.

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

## Exit

When the user runs:

```bash
opengpu exit
```

the CLI:

1. Marks the local config as disconnected.
2. Pauses contribution.
3. Leaves the identity and config in place for the next `start`.

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

## What It Does Not Do Yet

At this stage, the CLI does **not**:

1. Start a background daemon.
2. Talk to a live control plane.
3. Register the machine with a server.
4. Dispatch real jobs to remote nodes.

Those behaviors will come later when the control plane and node agent are online.
