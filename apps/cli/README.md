# opengpu CLI

Separately installable Rust command-line client for bootstrap, auth, node control, and updates.

Install with:

```bash
curl -fsSL https://novusx.ai/install | bash
```

Useful commands:

```bash
opengpu start
opengpu onboarding
opengpu cap
opengpu login
opengpu logout
opengpu status
opengpu nodes
opengpu exit
opengpu doctor
```

Startup flow:

- `opengpu start` creates local state if needed, connects locally when the secure device identity is available, and marks the machine ready
- `opengpu onboarding` shows the Mac-first contributor checklist until completed
- `opengpu cap` sets the contribution budget explicitly
- `opengpu login` stores a local operator bearer token
- `opengpu logout` clears that local token
- `opengpu connect` marks the machine ready once a cap has been recorded and the secure device identity is available
- `opengpu status` shows the live local routing decision

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/aigrid/docs/cli-startup-flow.md) for the full first-run sequence.
