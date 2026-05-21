# opengpu CLI

Separately installable Rust command-line client for bootstrap, auth, node control, and updates.

Install with:

```bash
curl -fsSL https://novusx.ai/install | bash
```

Useful commands:

```bash
opengpu start
opengpu login
opengpu status
opengpu nodes
opengpu exit
opengpu doctor
opengpu config show
opengpu config set backend cuda
```

Startup flow:

- `opengpu init` creates local state
- `opengpu start` creates local state if needed, connects locally, and marks the machine ready
- `opengpu login` stores auth locally
- `opengpu connect` marks the machine ready
- `opengpu status` shows the live local routing decision

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/aigrid/docs/cli-startup-flow.md) for the full first-run sequence.
