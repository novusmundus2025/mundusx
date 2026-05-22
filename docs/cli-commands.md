# OpenGPU CLI Commands

This is the current command surface for the Rust CLI.

## Core Commands

- `opengpu start` - create local state if needed, auto-detect the backend, connect locally, ask for contribution level in a vertical arrow-key menu, and print a startup summary with the public key
- `opengpu init` - create the local config and device identity
- `opengpu status` - show local state, detected backend, and local provider status
- `opengpu exit` - leave local contribution mode and pause the machine
- `opengpu model list` - show the local model cache and active model
- `opengpu model use <name>` - activate a cached model, or create a cache entry and activate it
- `opengpu model add <name>` - add a model to the local cache without switching to it
- `opengpu model remove <name>` - remove a cached model
- `opengpu model prune --yes` - remove inactive cached models
- `opengpu doctor` - inspect config paths and writability
- `opengpu logs` - show local log source information
- `opengpu update` - show update channel information

## Advanced Commands

These remain available, but they are hidden from the default `--help` output so the main CLI feels smaller and easier to learn:

- `opengpu login` - store local auth state
- `opengpu logout` - clear local auth state
- `opengpu connect` - mark the machine as ready
- `opengpu disconnect` - mark the machine as disconnected
- `opengpu nodes` - show the current sample node inventory
- `opengpu pause` - pause contribution
- `opengpu resume` - resume contribution
- `opengpu config path` - print the active config path
- `opengpu config show` - print the current config
- `opengpu config show --json` - print the config as JSON
- `opengpu config set control-plane-url <url>` - update the control plane URL
- `opengpu config set profile-name <name>` - update the local profile name
- `opengpu config set backend <auto|m|cuda>` - update the backend preference
- `opengpu config set device-id <id>` - override the local device ID
- `opengpu config set contribution-percent <1-100>` - set the contribution cap
- `opengpu config reset --yes` - delete local config files

## Notes

- The CLI currently operates on local state only.
- Real control-plane calls and node registration will come later.
- The active `opengpu start` session can be aborted with `Ctrl-C`, which rolls local state back to disconnected and paused.
- When backend preference is `auto`, `status` resolves the machine backend first and shows your machine as the active provider when connected.
- `nodes` still shows demo inventory from `apps/cli/src/nodes.rs`, but `status` no longer does.
- The model commands currently manage the local model cache manifest and active selection; real model downloads are still a future step.
