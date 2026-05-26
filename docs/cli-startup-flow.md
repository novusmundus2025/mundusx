# NovusX CLI Startup Flow

This document describes what the CLI does when a user starts using it for the first time and what each command is responsible for.

## First Run

The CLI does not start a long-running service by itself. Instead, it manages local state and prepares the machine to connect to the platform.

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

## Login

When the user runs:

```bash
opengpu login
```

the CLI:

1. Saves an operator bearer token locally.
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
5. Optionally overrides that with `--m`.
6. If no contribution cap is saved yet, prints a clear hint to run:
   - `opengpu cap`
   - the `cap` command opens the retro vertical selector for:
     - `20%` light
     - `30%` balanced
     - `50%` strong
     - `75%` aggressive
     - `90%` max
     - use the arrow keys and press Enter to confirm
     - press `Ctrl-C` to cancel the cap selector cleanly
7. If no active model is saved yet, caches a local model entry and marks it active.
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
12. Prints whether policy currently allows the Mac to accept work, including the power source, battery state, and identity readiness.
13. Keeps the reused secure device identity attached to the local config when available.

## Operator Auth

If the control plane starts with `OPENGPU_OPERATOR_TOKEN`, the human-facing API routes and browser health page require a matching bearer token. The CLI stores that token locally with `opengpu login`, and clears it with `opengpu logout`.

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

## Onboarding

When onboarding has not been completed yet, `opengpu start` and `opengpu connect` print a dedicated checklist panel and a hint to run:

```bash
opengpu onboarding --complete
```

The onboarding command only marks the review step as complete; it does not change the device identity or control-plane state.

When the contribution cap has not been saved yet, `opengpu cap` is the explicit command that records it before routing starts.

Those deeper network behaviors will come later when the control plane and node agent are online.
