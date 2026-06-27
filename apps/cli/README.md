# MundusX CLI

MundusX is the product; `opengpu` is the current CLI command.

Separately installable Rust command-line client for bootstrap, auth, node control, and updates.

Install with:

```bash
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
```

On Windows:

```powershell
.\install.ps1 -ReleaseBaseUrl http://127.0.0.1:8788/releases/latest/download
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
opengpu model import ./models/local-model.gguf --name local-model --backend cuda --vram-mb 4096
opengpu jobs submit --model <model> --prompt <text>
opengpu jobs status <job_id>
opengpu jobs wait <job_id> --timeout 300 --interval 2
```

Local no-auth smoke:

```bash
./scripts/no-auth-e2e-smoke.sh
```

Startup flow:

- For a fresh machine, review onboarding, set a contribution cap, and then run `opengpu start`; `opengpu install` guides the control-plane, cap, and model setup before start
- `opengpu start` creates local state if needed, connects locally when the secure device identity is available, and marks the machine ready
- `opengpu onboarding` shows the Mac-first contributor checklist until completed
- `opengpu cap` sets the contribution budget explicitly
- `opengpu update` prints the local install page and release preview URLs
- `opengpu login` stores a local operator bearer token
- `opengpu logout` clears that local token
- `opengpu connect` marks the machine ready once a cap has been recorded and the secure device identity is available
- `opengpu exit` leaves contribution mode; `opengpu disconnect` remains available as the explicit alias
- `opengpu status` shows the live local routing decision
- `opengpu model import` records contributor-supplied local GGUF files and reports whether they fit the selected backend and VRAM budget
- `opengpu jobs submit/status/wait` uses the async control-plane job API. `wait` is a CLI polling helper; the server still returns quickly and does not hold the request open.
- `scripts/no-auth-e2e-smoke.sh` verifies the local/UAT no-auth lifecycle: disabled operator auth on `/health`, signed node registration and heartbeat, no-auth job submit, node-agent claim and completion, and final CLI polling.

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/mundusx/docs/cli-startup-flow.md) for the full first-run sequence.
