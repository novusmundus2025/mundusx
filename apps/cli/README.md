# MundusX CLI

MundusX is the product; `opengpu` is the current CLI command.

Separately installable Rust command-line client for bootstrap, auth, node control, and updates.

Install with:

```bash
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
```

Apple Silicon macOS public release install. Replace `v0.1.10` with the current published Mac release version:

```bash
curl -fsSL https://raw.githubusercontent.com/mundusx/mundusx/uat/install.sh | RELEASE_BASE_URL=https://github.com/mundusx/mundusx/releases/download/cli-macos-v0.1.11 bash
```

On Windows local preview:

```powershell
.\install.ps1 -ReleaseBaseUrl http://127.0.0.1:8788/releases/latest/download -AllowUnsignedLocalPreview
```

Windows public release install uses the Windows-specific release:

```powershell
.\install.ps1 -ReleaseBaseUrl https://github.com/mundusx/mundusx/releases/download/cli-windows-v0.1.11
```

Useful commands:

```bash
opengpu install
opengpu onboarding
opengpu cap
opengpu start
opengpu login
opengpu logout
opengpu status
opengpu nodes
opengpu exit
opengpu doctor
opengpu model use
opengpu model add
opengpu model import ./models/local-model.gguf --name local-model --backend cuda --vram-mb 4096
opengpu run --prompt <text>
opengpu jobs submit --model <model> --prompt <text>
opengpu jobs status <job_id>
opengpu jobs wait <job_id> --timeout 300 --interval 2
```

Human-readable output supports cross-platform themes:

```bash
opengpu --theme reactor status
opengpu --theme classic doctor
NO_COLOR=1 opengpu status
```

- `reactor` uses branded red/gold accents when stdout is an interactive terminal.
- `classic` keeps plain, structured text for PowerShell, Git Bash, macOS/Linux terminals, and logs.
- `auto` is the default: it uses `reactor` on interactive terminals and falls back to `classic` for CI, non-TTY output, or `NO_COLOR`.
- JSON output stays machine-stable and unstyled for commands such as `opengpu doctor --json`, `opengpu status --json`, and `opengpu jobs wait --json`.

Local no-auth smoke:

```bash
./scripts/no-auth-e2e-smoke.sh
```

Startup flow:

- For a fresh machine, review onboarding, set a contribution cap, and then run `opengpu start`; `opengpu install` guides the control-plane, cap, and model setup before start
- `opengpu start` creates local state if needed, connects when the secure device identity is available, and keeps a foreground contribution session open until `Esc` or `Ctrl-C`
- `opengpu start --background` starts the node agent in daemon mode and returns after startup is verified
- `opengpu start --debug` runs the foreground contribution session with explicit diagnostic labeling
- `opengpu onboarding` shows the Mac-first contributor checklist until completed
- `opengpu cap` sets the contribution budget explicitly, with quick picks and custom whole-percent values up to 80%
- `opengpu update` prints the local install page and release preview URLs
- `opengpu login` stores a local operator bearer token; on Windows the token is protected with DPAPI outside `config.json`
- `opengpu logout` clears that local token and the protected Windows token blob
- `opengpu connect` marks the machine ready once a cap has been recorded and the secure device identity is available
- `opengpu exit` leaves contribution mode and stops the recorded background agent; `opengpu disconnect` remains available as the explicit alias
- `opengpu status` shows the live local routing decision
- `opengpu model use` and `opengpu model add` open the official model picker when no model name is passed
- `opengpu model import` records contributor-supplied local GGUF files and reports whether they fit the selected backend and VRAM budget
- `opengpu run` submits through the control-plane scheduler and waits for the result; the scheduler may assign the requestor node when it is the best eligible worker
- `opengpu jobs submit/status/wait` uses the async control-plane job API. `wait` is a CLI polling helper; the server still returns quickly and does not hold the request open.
- `scripts/no-auth-e2e-smoke.sh` verifies the local/UAT no-auth lifecycle: disabled operator auth on `/health`, signed node registration and heartbeat, no-auth job submit, node-agent claim and completion, and final CLI polling.

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/mundusx/docs/cli-startup-flow.md) for the full first-run sequence.
