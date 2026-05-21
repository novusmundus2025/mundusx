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
3. Sets default values for:
   - connection state
   - pause state
   - backend preference
   - control-plane URL
4. Stores the config in the preferred config directory, or falls back to a local `.opengpu/config.json` file if needed.

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
2. Marks the machine connected.
3. Clears the paused state.
4. Optionally sets `--m` or `--cuda`.
5. Prints a startup summary with:
   - device ID
   - platform
   - CPU core count
   - backend preference
   - connection state
   - pause state
   - config path
6. Saves the updated config.

## Connect

When the user runs:

```bash
opengpu connect
```

the CLI:

1. Marks the local config as connected.
2. Clears the paused state.
3. Prepares the machine to participate in routing once the control plane is online.

## Contribute

When the user runs:

```bash
opengpu contribute --m
```

or:

```bash
opengpu contribute --cuda
```

the CLI:

1. Updates the backend preference in local config.
2. Sets the machine’s target compute lane.
3. Uses that preference later when the CLI scores local sample nodes.

## Status

When the user runs:

```bash
opengpu status
```

the CLI:

1. Reads the current local config.
2. Loads the current sample node inventory.
3. Filters out offline nodes.
4. Scores the live nodes.
5. Prints the best local routing decision.

## What It Does Not Do Yet

At this stage, the CLI does **not**:

1. Start a background daemon.
2. Talk to a live control plane.
3. Register the machine with a server.
4. Dispatch real jobs to remote nodes.

Those behaviors will come later when the control plane and node agent are online.
