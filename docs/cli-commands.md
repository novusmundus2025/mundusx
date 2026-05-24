# OpenGPU CLI Commands

This is the current command surface for the Rust CLI.

## Core Commands

- `opengpu start` - create local state if needed, auto-detect the backend, connect locally when the secure device identity is available, and print a startup summary with the public key; if no cap is saved, it prints a hint to run `opengpu cap`
- `opengpu onboarding` - review the contributor onboarding checklist
- `opengpu onboarding --complete` - mark onboarding complete after review
- `opengpu onboarding --reset` - reopen the onboarding checklist
- `opengpu cap` - choose the contribution budget explicitly
- `opengpu cap --percent <value>` - save a contribution budget directly
- `opengpu cap --reset` - clear the saved contribution budget
- `opengpu status` - show local state, detected backend, local provider status, and local policy readiness
- `opengpu exit` - leave local contribution mode and pause the machine
- `opengpu model list` - show the local model cache and active model
- `opengpu model use <name>` - activate a cached model, or create a cache entry and activate it
- `opengpu model add <name>` - add a model to the local cache without switching to it
- `opengpu model remove <name>` - remove a cached model
- `opengpu model prune --yes` - remove inactive cached models
- `opengpu doctor` - inspect config paths and writability
- `opengpu logs` - show local log source information
- `opengpu update` - show the local install page and release preview URLs

## Advanced Commands

These remain available, but they are hidden from the default `--help` output so the main CLI feels smaller and easier to learn:

- `opengpu login --token <token>` - store local operator auth state
- `opengpu login` - prompt for and store local operator auth state
- `opengpu logout` - clear local operator auth state
- `opengpu connect` - mark the machine as ready
- `opengpu disconnect` - mark the machine as disconnected
- `opengpu nodes` - show the current sample node inventory
- `opengpu pause` - pause contribution
- `opengpu resume` - resume contribution

## Notes

- The CLI currently operates on local state only.
- Real control-plane calls and node registration will come later.
- The active `opengpu start` session can be aborted with `Ctrl-C`, which rolls local state back to disconnected and paused.
- When backend preference is `auto`, `status` resolves the machine backend first and shows your machine as the active provider when connected and policy allows it.
- `nodes` still shows demo inventory from `apps/cli/src/nodes.rs`, but `status` no longer does.
- `status` also reports `powerSource`, `onBattery`, `batteryPercent`, `identityTrustPath`, `policyAllowed`, and `policyReason` so you can see why the Mac is paused or quiet, and whether it is using Keychain or the local encrypted fallback.
- `login` and `logout` manage the local operator bearer token used for the control-plane API when operator auth is enabled.
- The model commands currently manage the local model cache manifest and active selection; real model downloads are still a future step.
- `onboarding` is a local contributor review step that summarizes the secure device identity, hostname, model, policy, and credits setup; `start` prints it automatically until it is marked complete.
- `cap` is the explicit command for choosing the Mac contribution budget before the node is treated as ready for routing.
- `start` only marks the node ready when the secure device identity is available.
- Config inspection now happens through `status` and `doctor`; dedicated `config` subcommands are not part of the current CLI surface.
