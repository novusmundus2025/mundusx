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
curl -fsSL https://opengpu.ai/install | bash
```

That installer will download the prebuilt binary for the user's operating system and CPU architecture.

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

The CLI will eventually support:

```bash
opengpu init
opengpu login
opengpu connect
opengpu status
opengpu contribute --m
opengpu contribute --cuda
opengpu pause
opengpu resume
opengpu logs
opengpu update
```
