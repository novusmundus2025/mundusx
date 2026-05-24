# OpenGPU

OpenGPU is a distributed compute network that routes requests to the best available live node across:

- Apple Silicon `M` series nodes

OpenGPU is the product name. NovusX is the company name for the future public install domain, but the current flow is localhost-only.

## Repo Shape

This repository is a monorepo for the core platform:

- `apps/cli` - separately installable command-line client
- `apps/control-plane` - scheduler, routing, auth, and job management
- `apps/dashboard` - web UI for operators and users
- `agents/node` - local node agent/daemon
- `workers/m-series` - Apple Silicon execution backend
- `packages/shared` - shared utilities and types
- `packages/proto` - protobuf and RPC contracts

Rust workspace:

- `Cargo.toml` at the repo root
- `apps/cli` is the first compiled Rust crate

## One-Click Install

Users should install the CLI from the localhost release preview and never need Rust locally:

```bash
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
```

That installer will download the matching release binary for the user's operating system and CPU architecture from the local release preview, with the current release channel focused on Apple Silicon Macs, then verify the checksum when available.

See [docs/install-page.md](/Users/DBATALL/Documents/aigrid/docs/install-page.md) for the localhost install wording and [docs/install-strategy.md](/Users/DBATALL/Documents/aigrid/docs/install-strategy.md) for the distribution plan.

Release builds for the CLI are published from GitHub Actions on `cli-v*` tags.

## First Implementation Files

- `apps/cli/src/main.rs` for the Rust CLI entrypoint
- `apps/cli/src/routing.rs` for local selection scoring
- `agents/node/src/main.rs` for the Rust node agent entrypoint
- `agents/node/src/worker.rs` for the local worker subprocess scaffold
- `apps/control-plane/src/main.rs` for the Rust control-plane entrypoint
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

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/aigrid/docs/cli-startup-flow.md) for the exact first-run and command-by-command behavior.

## Docs

- [LICENSE](/Users/DBATALL/Documents/aigrid/LICENSE) for the Apache-2.0 licensed portions of the repo
- [apps/control-plane/PROPRIETARY_LICENSE.md](/Users/DBATALL/Documents/aigrid/apps/control-plane/PROPRIETARY_LICENSE.md) for the private control-plane subtree
- [docs/cli-commands.md](/Users/DBATALL/Documents/aigrid/docs/cli-commands.md) for the current CLI command reference
- [docs/missing-items.md](/Users/DBATALL/Documents/aigrid/docs/missing-items.md) for the work that is still missing
- [docs/master-checklist.md](/Users/DBATALL/Documents/aigrid/docs/master-checklist.md) for the single working checklist we track item by item
- [docs/pending-items.md](/Users/DBATALL/Documents/aigrid/docs/pending-items.md) for the next concrete implementation steps
- [docs/heartbeat-flow.md](/Users/DBATALL/Documents/aigrid/docs/heartbeat-flow.md) for the proposed node heartbeat timing and offline thresholds
- [docs/system-flow.md](/Users/DBATALL/Documents/aigrid/docs/system-flow.md) for the living end-to-end architecture diagram
- [docs/repo-standards.md](/Users/DBATALL/Documents/aigrid/docs/repo-standards.md) for folder structure and review rules
- [docs/governance-model.md](/Users/DBATALL/Documents/aigrid/docs/governance-model.md) for the federated company / standards org trust model
- [docs/runtime-transport.md](/Users/DBATALL/Documents/aigrid/docs/runtime-transport.md) for the local-process versus HTTP-server map
- [docs/device-identity-lifecycle.md](/Users/DBATALL/Documents/aigrid/docs/device-identity-lifecycle.md) for the recommended secure device key lifecycle
- [docs/model-lifecycle.md](/Users/DBATALL/Documents/aigrid/docs/model-lifecycle.md) for contributor-side model cache, switch, and prune rules
- [docs/install-strategy.md](/Users/DBATALL/Documents/aigrid/docs/install-strategy.md) for the cross-platform distribution plan
- [docs/install-page.md](/Users/DBATALL/Documents/aigrid/docs/install-page.md) for the local install page copy and flow
- [scripts/localhost-smoke.sh](/Users/DBATALL/Documents/aigrid/scripts/localhost-smoke.sh) for the one-shot localhost install and docs smoke test
- `npm run smoke:local` for the same one-shot localhost check from the repo root
- [scripts/local-release-preview.sh](/Users/DBATALL/Documents/aigrid/scripts/local-release-preview.sh) for the repo-managed localhost release preview helper
- [docs/public-docs-site.md](/Users/DBATALL/Documents/aigrid/docs/public-docs-site.md) for the local docs site preview at `http://127.0.0.1:<port>/docs`
- [docs/agent-worker-contract.md](/Users/DBATALL/Documents/aigrid/docs/agent-worker-contract.md) for the agent and worker handshake contract
- [docs/node-agent.md](/Users/DBATALL/Documents/aigrid/docs/node-agent.md) for the current node agent prototype and local state files
- [docs/control-plane.md](/Users/DBATALL/Documents/aigrid/docs/control-plane.md) for the current control plane prototype and endpoints
- [docs/dashboard.md](/Users/DBATALL/Documents/aigrid/docs/dashboard.md) for the local operator dashboard
- [docs/credits-model.md](/Users/DBATALL/Documents/aigrid/docs/credits-model.md) for the append-only credits ledger and reward rule
- [docs/onboarding.md](/Users/DBATALL/Documents/aigrid/docs/onboarding.md) for the contributor onboarding checklist and state
- [docs/supabase-schema.md](/Users/DBATALL/Documents/aigrid/docs/supabase-schema.md) for the company-side Supabase schema sketch
- [docs/supabase-migrations.md](/Users/DBATALL/Documents/aigrid/docs/supabase-migrations.md) for the migration runner and apply flow
- [supabase/schema.sql](/Users/DBATALL/Documents/aigrid/supabase/schema.sql) for the SQL you apply in Supabase
- [supabase/migrations/0001_rls.sql](/Users/DBATALL/Documents/aigrid/supabase/migrations/0001_rls.sql) for the RLS rollout
- [docs/worker.md](/Users/DBATALL/Documents/aigrid/docs/worker.md) for the current worker prototype and launch contract
