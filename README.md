# NovusX

NovusX is the product and platform we are building here. `opengpu` is the current CLI command name and compatibility binary, while the broader public OpenGPU ecosystem is treated as separate.

This repository contains the public, contributor-facing pieces of NovusX. The operator/control-plane side lives in a separate private repo.

NovusX is a distributed compute network that routes requests to the best available live node across Apple Silicon `M` series nodes.

`opengpu` is the current command family for the local prototype, but the current flow is localhost-only.

## Repo Shape

This repository is a monorepo for the public core platform:

- `apps/cli` - separately installable command-line client
- `agents/node` - local node agent/daemon
- `workers/m-series` - Apple Silicon execution backend
- `packages/shared` - shared utilities and types
- `packages/proto` - protobuf and RPC contracts
- `tools` - macOS identity helpers and build-time support
- `scripts` - install/bootstrap and release helper scripts

The private operator repo owns the control plane, dashboard, and company-side database artifacts.

Rust workspace:

- `Cargo.toml` at the repo root
- `apps/cli` is the first compiled Rust crate

## One-Click Install

Users should install the CLI from the localhost release preview and never need Rust locally:

```bash
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
```

That installer will download the matching release binary for the user's operating system and CPU architecture from the local release preview, with the current release channel focused on Apple Silicon Macs, then verify the checksum when available.

See [docs/install-page.md](/Users/DBATALL/Documents/mundusx/docs/install-page.md) for the localhost install wording and [docs/install-strategy.md](/Users/DBATALL/Documents/mundusx/docs/install-strategy.md) for the distribution plan.
The local install page also exposes a machine-readable manifest at `http://127.0.0.1:3002/install.json` for tooling and future public rollout work. The same dashboard also mirrors the future public-endpoint shape at `http://127.0.0.1:3002/public/install` and `http://127.0.0.1:3002/public/install.json`. The matching local release preview is also manifest-driven and serves `release-manifest.json` from `http://127.0.0.1:8788/releases/latest/download/`.
For a repo-owned public docs artifact, run `npm run build:docs-site` to generate the static site into `dist/public-docs-site`. That same build is what the GitHub Pages workflow deploys from `uat` and `main`.

Release builds for the CLI are published from GitHub Actions on `cli-v*` tags with checksums and a signed manifest attached to each release artifact set.

## First Implementation Files

- `apps/cli/src/main.rs` for the Rust CLI entrypoint
- `apps/cli/src/routing.rs` for local selection scoring
- `agents/node/src/main.rs` for the Rust node agent entrypoint
- `agents/node/src/worker.rs` for the local worker subprocess scaffold
- `install.sh` for one-click binary installation
- `packages/shared/src/index.ts` for cross-package types
- `packages/proto/schema/opengpu.proto` for the wire contract

## MVP Goal

Route each request to the most suitable live node, rather than combining partial outputs from multiple machines.

## First Commands

The CLI now supports local setup, node inspection, and preference management:

```bash
opengpu start
opengpu cap
opengpu status
opengpu exit
opengpu model list
opengpu model use <name>
opengpu model add <name>
opengpu model remove <name>
opengpu model prune --yes
opengpu doctor
opengpu logs
opengpu update
```

## CLI Startup Flow

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/mundusx/docs/cli-startup-flow.md) for the exact first-run and command-by-command behavior.

## Docs

- [LICENSE](/Users/DBATALL/Documents/mundusx/LICENSE) for the Apache-2.0 licensed portions of the repo
- [docs/cli-commands.md](/Users/DBATALL/Documents/mundusx/docs/cli-commands.md) for the current CLI command reference
- [docs/missing-items.md](/Users/DBATALL/Documents/mundusx/docs/missing-items.md) for the work that is still missing
- [docs/master-checklist.md](/Users/DBATALL/Documents/mundusx/docs/master-checklist.md) for the single working checklist we track item by item
- [docs/gap-checklist.md](/Users/DBATALL/Documents/mundusx/docs/gap-checklist.md) for the compare-and-contrast list of what is already implemented versus what is still missing
- [docs/pending-items.md](/Users/DBATALL/Documents/mundusx/docs/pending-items.md) for the next concrete implementation steps
- [docs/heartbeat-flow.md](/Users/DBATALL/Documents/mundusx/docs/heartbeat-flow.md) for the proposed node heartbeat timing and offline thresholds
- [docs/system-flow.md](/Users/DBATALL/Documents/mundusx/docs/system-flow.md) for the living end-to-end architecture diagram
- [docs/repo-standards.md](/Users/DBATALL/Documents/mundusx/docs/repo-standards.md) for folder structure and review rules
- [docs/naming-strategy.md](/Users/DBATALL/Documents/mundusx/docs/naming-strategy.md) for the product / command naming rules
- [docs/governance-model.md](/Users/DBATALL/Documents/mundusx/docs/governance-model.md) for the federated company / standards org trust model
- [docs/requestor-routing.md](/Users/DBATALL/Documents/mundusx/docs/requestor-routing.md) for org gateway versus direct provider routing
- [docs/runtime-transport.md](/Users/DBATALL/Documents/mundusx/docs/runtime-transport.md) for the local-process versus HTTP-server map
- [docs/job-eligibility.md](/Users/DBATALL/Documents/mundusx/docs/job-eligibility.md) for the ready / busy / paused / offline lifecycle and what happens after a job completes
- [docs/device-identity-lifecycle.md](/Users/DBATALL/Documents/mundusx/docs/device-identity-lifecycle.md) for the recommended secure device key lifecycle
- [docs/model-lifecycle.md](/Users/DBATALL/Documents/mundusx/docs/model-lifecycle.md) for contributor-side model cache, switch, and prune rules
- [docs/install-strategy.md](/Users/DBATALL/Documents/mundusx/docs/install-strategy.md) for the cross-platform distribution plan
- [docs/install-page.md](/Users/DBATALL/Documents/mundusx/docs/install-page.md) for the local install page copy and flow
- `npm run build:docs-site` to generate the static public docs site preview from tracked markdown
- [scripts/localhost-smoke.sh](/Users/DBATALL/Documents/mundusx/scripts/localhost-smoke.sh) for the one-shot localhost install and docs smoke test
- `npm run smoke:local` for the same one-shot localhost check from the repo root
- [scripts/local-release-preview.sh](/Users/DBATALL/Documents/mundusx/scripts/local-release-preview.sh) for the repo-managed localhost release preview helper
- [scripts/verify-release-packaging.sh](/Users/DBATALL/Documents/mundusx/scripts/verify-release-packaging.sh) for the release artifact packaging validator
- [docs/public-docs-site.md](/Users/DBATALL/Documents/mundusx/docs/public-docs-site.md) for the local docs site preview at `http://127.0.0.1:<port>/docs` and the public-endpoint mirror at `http://127.0.0.1:<port>/public/docs`
- [docs/agent-worker-contract.md](/Users/DBATALL/Documents/mundusx/docs/agent-worker-contract.md) for the agent and worker handshake contract
- [docs/requestor-flow.md](/Users/DBATALL/Documents/mundusx/docs/requestor-flow.md) for the subscriber/requestor job submission and response flow
- [docs/requestor-api.md](/Users/DBATALL/Documents/mundusx/docs/requestor-api.md) for the OpenAI / OneAPI-compatible request intake adapter
- [docs/retrieval-policy.md](/Users/DBATALL/Documents/mundusx/docs/retrieval-policy.md) for when to answer from the model, when to retrieve local context, and when to use live tools
- [docs/node-agent.md](/Users/DBATALL/Documents/mundusx/docs/node-agent.md) for the current node agent prototype and local state files
- [docs/worker.md](/Users/DBATALL/Documents/mundusx/docs/worker.md) for the current worker prototype and launch contract
