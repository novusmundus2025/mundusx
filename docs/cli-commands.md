# MundusX CLI Commands

This is the current command surface for the Rust CLI.

Contributor-facing commands must stay aligned with the app surface. See
[Contributor App Parity](contributor-app-parity.md) for the rule that ordinary
contributors should not need PowerShell for normal setup, model, status, start,
pause, resume, or disconnect workflows.

## Core Commands

- `opengpu start` - create local state if needed, auto-detect the backend, ask for a contribution cap on interactive first run, connect locally when the secure device identity is available, print a startup summary with the public key, and keep a foreground contribution session open
- `opengpu start --background` - start the node agent in daemon mode and return after startup is verified
- `opengpu install` - run the guided machine setup wizard, choose public MundusX or private/custom control plane, save a community contribution cap, show only models that fit that cap, and prepare the node for `opengpu start`
- `opengpu onboarding` - review the contributor onboarding checklist
- `opengpu onboarding --complete` - mark onboarding complete after review
- `opengpu onboarding --reset` - reopen the onboarding checklist
- `opengpu cap` - choose the contribution budget explicitly
- `opengpu cap --percent <value>` - save a contribution budget directly
- `opengpu cap --reset` - clear the saved contribution budget
- `opengpu status` - show local state, detected backend, local provider status, and local policy readiness
- `opengpu exit` - leave local contribution mode, pause the machine, and cool any warm GPU runtime
- `opengpu model list` - show the local model cache and active model
- `opengpu model use` - open the official model picker, download/cache the selected model, and mark it active
- `opengpu model use <name>` - activate a cached model, or download an official open preset first and then activate it
- `opengpu model add` - open the official model picker and download/cache the selected model without switching active model
- `opengpu model add <name>` - add a model to the local cache, downloading an official open preset first when available
- `opengpu model import <path> --name <name> --backend cuda --vram-mb <mb>` - record an existing local model file with format, quantization, size, and compatibility metadata
- `opengpu model remove <name>` - remove a cached model
- `opengpu model prune --yes` - remove inactive cached models
- `opengpu cluster scan` - probe the well-known local ports for a running LLM cluster and show what answered
- `opengpu cluster use <url>` - contribute a running local cluster without waiting for the setup prompt
- `opengpu cluster forget` - stop contributing the recorded cluster and allow the setup prompt again
- `opengpu doctor` - inspect config paths, writability, CUDA prerequisite state, low-VRAM profile, Windows LM Studio runtime guidance, and opt-in Linux vLLM readiness
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
- `opengpu pause` - keep the device connected, stop new job claims, and let the running node agent cool the warm `llama-server` so its GPU allocation is released
- `opengpu resume` - restart contribution in the background, re-run readiness checks, and warm the configured model before accepting work

## Notes

- The CLI currently operates on local state only.
- Real control-plane calls and node registration will come later.
- The active `opengpu start` foreground session can be stopped with `Esc` or `Ctrl-C`, which rolls local state back to disconnected and paused, stops the node agent, and releases any warm `llama-server` GPU runtime.
- When backend preference is `auto`, `status` resolves the machine backend first and shows your machine as the active provider when connected and policy allows it.
- `vllm` is explicit opt-in for Linux/Ubuntu NVIDIA nodes. `auto` does not select it, and Windows continues to use the llama.cpp CUDA path.
- `nodes` still shows demo inventory from `apps/cli/src/nodes.rs`, but `status` no longer does.
- `status` and `start` also report `powerSource`, `onBattery`, `batteryPercent`, `identityTrustPath`, `policyAllowed`, `policyReason`, `readyForJobs`, `readinessReason`, and `contributedCluster` so a connected node can still explain why it is not schedulable yet.
- When a cluster is contributed, its model is the node's effective active model: `status`, `doctor`, the startup summary, and the default model for `run` and `jobs submit` all resolve to it, and the local model cache stays empty.
- `readyForJobs: yes` means the node is connected, not paused, has secure identity, has a selected active model, passes local power policy, and is not using a model manifest marked `rejected`.
- The foreground node agent also prints the advertised registration capability at connect time, including `readyForJobs`, `readinessReason`, `runtimeMode`, active model, and VRAM budget when available.
- `login` and `logout` manage the local operator bearer token used for the control-plane API when operator auth is enabled. On Windows, `login` stores the token in a DPAPI-protected blob outside `config.json`, and `logout` removes that protected token.
- The model commands manage the local model cache manifest and active selection, download official open presets from the reviewed catalog before caching or activating them, and can import contributor-supplied local GGUF files with compatibility metadata for node capability reporting.
- Bare `model use` and `model add` show an arrow-key official model picker with provider/source URL, backend compatibility, estimated VRAM, and cap-fit context; passing an exact name remains available for scripts.
- Official catalog entries must include GGUF format, backend compatibility, and conservative estimated VRAM metadata before review. CUDA entries should be sized against the cap-applied budget, so a 4 GB card at an 80% cap only sees models estimated at 3.2 GB VRAM or less; entries without VRAM metadata are not offered for CUDA auto-download.
- `onboarding` is a local contributor review step that summarizes the secure device identity, hostname, model, policy, and credits setup; `start` prints it automatically until it is marked complete.
- `start` asks for the contribution budget on interactive first run; `cap` is the explicit command for changing it later.
- Community contribution quick picks are `20%`, `30%`, `50%`, `65%`, and `80%`; custom caps can be any whole percent from `1%` through `80%`.
- `install` is the guided setup command after the binary is installed. Public mode saves the hosted MundusX control plane; private mode asks for a full custom URL; blank URL means public. The wizard also asks for the model and refuses choices that do not fit the selected contribution cap and detected machine capacity.
- MundusX-managed runtimes do not ask contributors to guess a concurrency value. The node derives a safe ceiling from the cap-applied memory and selected model; CUDA budgets up to 8 GB are fixed to one active job and additional requests wait in the control-plane queue. A configured limit may lower this ceiling but cannot raise it.
- `install` and `start` probe the machine for a local LLM cluster that is already running (Ollama on `11434`, LM Studio on `1234`, vLLM on `8000`, and any OpenAI-compatible server on `8080`). When one answers with at least one model, the contributor is asked whether to contribute that running cluster instead of provisioning a MundusX runtime and downloading another copy of the weights. The MundusX-managed `llama-server` port `8789` is never treated as a foreign cluster.
- The cluster question lives on the contribution level menu as a final `Clusters detected (N)` row. Choosing it lists every running cluster, biggest model first, plus a `None` row; picking one contributes it, saves the default cap for local policy, and skips model selection entirely. `None` or `Esc` returns to the contribution level menu.
- The runtime is identified from its own model listing (`owned_by`, llama.cpp `meta` fields, Ollama `details`/`digest`), not from the port, so a llama.cpp server on `8000` is reported as llama.cpp rather than vLLM.
- A contributed cluster advertises `runtime_mode: contributed-cluster`, a capacity class derived from the advertised model's size, no host-derived VRAM budget, and parallel slots bounded by the runtime's reported scheduler/KV capacity. A contributor may choose a lower limit but cannot exceed that runtime ceiling. Heavy/synthesis clusters receive `reducer`/`synthesizer` roles.
- Clusters and models are ranked by size, not probe order: a node advertises the largest model its cluster serves, and the cluster serving the largest model wins across endpoints. Parameter count decides first (llama.cpp `meta.n_params`, Ollama `details.parameter_size`), with on-disk bytes breaking ties. `cluster use --model <name>` overrides the automatic pick.
- `cluster scan` lists servable endpoints biggest first, then any that are running with nothing loaded.
- Answering yes records the cluster in `config.json`, skips the MundusX model selector, and skips runtime provisioning. Answering no is remembered so setup stops asking; `opengpu cluster use <url>` or `--contribute-cluster` reopens the decision.
- `install --contribute-cluster` / `start --contribute-cluster` contributes a detected cluster without prompting, and `--no-contribute-cluster` always declines, so scripted installs never block on the question. `--cluster-url <url>` probes one endpoint instead of the well-known ports.
- Non-interactive runs never contribute a cluster silently: without a flag they print the detection and the `--contribute-cluster` hint, then continue with normal MundusX setup.
- `OPENGPU_CLUSTER_PROBE_URLS` replaces the default probe list with a comma-separated set of base URLs, and `OPENGPU_SKIP_CLUSTER_DETECT=1` turns detection off entirely.
- A detected endpoint that is running but advertises no model is reported as detected-but-idle and is not offered for contribution, because it cannot serve work yet.
- `start` only marks the node ready when the secure device identity is available.
- Config inspection now happens through `status` and `doctor`; dedicated `config` subcommands are not part of the current CLI surface.
