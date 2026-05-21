# OpenGPU

OpenGPU is a distributed compute network that routes requests to the best available live node across:

- Apple Silicon `M` series nodes
- NVIDIA `CUDA` nodes

## Repo Shape

This repository is a monorepo for the core platform:

- `apps/cli` - separately installable command-line client
- `apps/control-plane` - scheduler, routing, auth, and job management
- `apps/dashboard` - web UI for operators and users
- `agents/node` - local node agent/daemon
- `workers/m-series` - Apple Silicon execution backend
- `workers/cuda` - NVIDIA execution backend
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
- `install.sh` for one-click binary installation
- `packages/shared/src/index.ts` for cross-package types
- `packages/proto/schema/opengpu.proto` for the wire contract
- `apps/control-plane/src/main.js` for the service skeleton
- `agents/node/src/main.js` for the local node daemon skeleton

## MVP Goal

Route each request to the most suitable live node, rather than combining partial outputs from multiple machines.

## First Commands

The CLI now supports local setup, node inspection, and preference management:

```bash
opengpu start
opengpu init
opengpu login
opengpu logout
opengpu connect
opengpu disconnect
opengpu status
opengpu nodes
opengpu contribute --m
opengpu contribute --cuda
opengpu pause
opengpu resume
opengpu doctor
opengpu config path
opengpu config show
opengpu config set control-plane-url https://api.novusx.ai
opengpu config set profile-name novus
opengpu config set backend cuda
opengpu config reset --yes
opengpu logs
opengpu update
```

## CLI Startup Flow

See [docs/cli-startup-flow.md](/Users/DBATALL/Documents/aigrid/docs/cli-startup-flow.md) for the exact first-run and command-by-command behavior.

## Docs

- [docs/cli-commands.md](/Users/DBATALL/Documents/aigrid/docs/cli-commands.md) for the current CLI command reference
- [docs/missing-items.md](/Users/DBATALL/Documents/aigrid/docs/missing-items.md) for the work that is still missing
