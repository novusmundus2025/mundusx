# OpenGPU

OpenGPU is a distributed compute network that routes requests to the best available live node across:

- Apple Silicon `M` series nodes

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

Users should install the CLI with a single command and never need Rust locally:

```bash
curl -fsSL https://novusx.ai/install | bash
```

That installer will download the prebuilt binary for the user's operating system and CPU architecture.

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
opengpu init
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
- [docs/pending-items.md](/Users/DBATALL/Documents/aigrid/docs/pending-items.md) for the next concrete implementation steps
- [docs/heartbeat-flow.md](/Users/DBATALL/Documents/aigrid/docs/heartbeat-flow.md) for the proposed node heartbeat timing and offline thresholds
- [docs/system-flow.md](/Users/DBATALL/Documents/aigrid/docs/system-flow.md) for the living end-to-end architecture diagram
- [docs/repo-standards.md](/Users/DBATALL/Documents/aigrid/docs/repo-standards.md) for folder structure and review rules
- [docs/governance-model.md](/Users/DBATALL/Documents/aigrid/docs/governance-model.md) for the federated company / standards org trust model
- [docs/runtime-transport.md](/Users/DBATALL/Documents/aigrid/docs/runtime-transport.md) for the local-process versus HTTP-server map
- [docs/device-identity-lifecycle.md](/Users/DBATALL/Documents/aigrid/docs/device-identity-lifecycle.md) for the recommended secure device key lifecycle
- [docs/model-lifecycle.md](/Users/DBATALL/Documents/aigrid/docs/model-lifecycle.md) for contributor-side model cache, switch, and prune rules
- [docs/install-strategy.md](/Users/DBATALL/Documents/aigrid/docs/install-strategy.md) for the cross-platform distribution plan
- [docs/agent-worker-contract.md](/Users/DBATALL/Documents/aigrid/docs/agent-worker-contract.md) for the agent and worker handshake contract
- [docs/node-agent.md](/Users/DBATALL/Documents/aigrid/docs/node-agent.md) for the current node agent prototype and local state files
- [docs/control-plane.md](/Users/DBATALL/Documents/aigrid/docs/control-plane.md) for the current control plane prototype and endpoints
- [docs/supabase-schema.md](/Users/DBATALL/Documents/aigrid/docs/supabase-schema.md) for the company-side Supabase schema sketch
- [docs/supabase-migrations.md](/Users/DBATALL/Documents/aigrid/docs/supabase-migrations.md) for the migration runner and apply flow
- [supabase/schema.sql](/Users/DBATALL/Documents/aigrid/supabase/schema.sql) for the SQL you apply in Supabase
- [supabase/migrations/0001_rls.sql](/Users/DBATALL/Documents/aigrid/supabase/migrations/0001_rls.sql) for the RLS rollout
- [docs/worker.md](/Users/DBATALL/Documents/aigrid/docs/worker.md) for the current worker prototype and launch contract
