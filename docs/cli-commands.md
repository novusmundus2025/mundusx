# MundusX CLI Commands

This is the current command surface for the Rust CLI.

## Core Commands

- `opengpu start` - create local state if needed, auto-detect the backend, ask for a contribution cap on interactive first run, connect locally when the secure device identity is available, and print a startup summary with the public key
- `opengpu install` - run the guided machine setup wizard, choose public MundusX or private/custom control plane, save a community contribution cap, show only models that fit that cap, and prepare the node for `opengpu start`
- `opengpu onboarding` - review the contributor onboarding checklist
- `opengpu onboarding --complete` - mark onboarding complete after review
- `opengpu onboarding --reset` - reopen the onboarding checklist
- `opengpu cap` - choose the contribution budget explicitly
- `opengpu cap --percent <value>` - save a contribution budget directly
- `opengpu cap --reset` - clear the saved contribution budget
- `opengpu status` - show local state, detected backend, local provider status, and local policy readiness
- `opengpu exit` - leave local contribution mode and pause the machine
- `opengpu model list` - show the local model cache and active model
- `opengpu model use <name>` - activate a cached model, or download an official open preset first and then activate it
- `opengpu model add <name>` - add a model to the local cache, downloading an official open preset first when available
- `opengpu model import <path> --name <name> --backend cuda --vram-mb <mb>` - record an existing local model file with format, quantization, size, and compatibility metadata
- `opengpu model remove <name>` - remove a cached model
- `opengpu model prune --yes` - remove inactive cached models
- `opengpu doctor` - inspect config paths, writability, CUDA prerequisite state, low-VRAM profile, and Windows LM Studio runtime guidance
- `opengpu logs` - show local log source information
- `opengpu update` - show the local install page and release preview URLs

## Advanced Commands

These remain available, but they are hidden from the default `--help` output so the main CLI feels smaller and easier to learn:

- `opengpu login --token <token>` - store local operator auth state
- `opengpu login` - prompt for and store local operator auth state
- `opengpu logout` - clear local operator auth state
- `opengpu connect` - mark the machine as ready
- `opengpu disconnect` - same behavior as `opengpu exit`, kept as the explicit advanced form
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
- `login` and `logout` manage the local operator bearer token used for the control-plane API when operator auth is enabled. On Windows, `login` stores the token in a DPAPI-protected blob outside `config.json`, and `logout` removes that protected token.
- The model commands manage the local model cache manifest and active selection, download official open presets from the reviewed catalog before caching or activating them, and can import contributor-supplied local GGUF files with compatibility metadata for node capability reporting.
- Official catalog entries must include GGUF format, backend compatibility, and conservative estimated VRAM metadata before review. CUDA entries should be sized against the cap-applied budget, so a 4 GB card at an 80% cap only sees models estimated at 3.2 GB VRAM or less; entries without VRAM metadata are not offered for CUDA auto-download.
- `onboarding` is a local contributor review step that summarizes the secure device identity, hostname, model, policy, and credits setup; `start` prints it automatically until it is marked complete.
- `start` asks for the contribution budget on interactive first run; `cap` is the explicit command for changing it later.
- Community contribution caps are limited to `20%`, `30%`, `50%`, `65%`, and `80%`.
- `install` is the guided setup command after the binary is installed. Public mode saves the hosted MundusX control plane; private mode asks for a full custom URL; blank URL means public. The wizard also asks for the model and refuses choices that do not fit the selected contribution cap and detected machine capacity.
- `start` only marks the node ready when the secure device identity is available.
- Config inspection now happens through `status` and `doctor`; dedicated `config` subcommands are not part of the current CLI surface.
