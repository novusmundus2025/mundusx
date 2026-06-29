# Who Runs What

## As a User (Node Contributor)

You install one thing and run one command:

```
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
opengpu start
```

### What runs on your machine

| Component | Binary | What it does |
|---|---|---|
| **CLI** | `opengpu` | The only thing you interact with. Handles connect, disconnect, status, update. |
| **Node Agent** | started by CLI | Runs in the background. Sends heartbeats, reports capability, picks up and runs jobs. |
| **Worker** | spawned per job | Short-lived subprocess launched by the agent when a job is assigned. Runs the actual model inference. |

### What you do NOT run

- Control plane (that's in the private operator repo)
- Dashboard (that's in the private operator repo)
- Any database or server

### Your machine's role

Your machine sits idle most of the time. When a job is routed to you:
1. The node agent claims it
2. A worker subprocess launches, runs inference, returns the result
3. Worker exits, agent goes back to idle

---

## As the Company (MundusX)

You run and maintain the shared infrastructure that all nodes connect to.

### Components you operate

| Component | Stack | What it does |
|---|---|---|
| **Control Plane** | Rust (HTTP API) | Node registry, heartbeat ingestion, job queue, routing decisions, job tracking |
| **Dashboard** | Node.js | Live view of network health, node status, job history, credits. Lives in the private operator repo. |
| **Install endpoint** | Local dashboard preview | `http://127.0.0.1:<port>/install` — current landing page and install command for the signed binary preview |
| **Release pipeline** | GitHub Actions + localhost preview | Builds, verifies checksums, signs the release manifest, and publishes signed binaries on every `cli-v*` tag (Mac-first release channel today), while the current preview uses a localhost release source |
| **Auth service** | Local bearer token + signed device requests | Issues operator tokens locally for the prototype and verifies device signatures |
| **Credits ledger** | Supabase-backed ledger | Tracks contribution and usage accounting per node in the private operator repo |
| **Durable state store** | Supabase / Postgres | Persistent DB behind the control plane, with the local JSON cache kept only as a fallback |

### What "maintained" means per component

**Control Plane**
- Keep the API up (`/v1/register`, `/v1/heartbeat`, `/v1/jobs/*`)
- Monitor for nodes going stale (missed heartbeats)
- Scale as node count grows
- Migrate from in-memory state to a real DB (top priority)

**Install endpoint**
- Serve the local install page reliably — this is the user's first touch point during development
- Keep the install command, checksums, and release links in sync with the current Mac-first localhost preview

**Release pipeline**
- Tag `cli-v*` triggers a Mac-first release build today
- Current release channel targets Apple Silicon macOS binaries
- Binaries are checksum-verified in CI and their release manifest is signed before the install script points to them

**Dashboard**
- Lives in the private operator repo and renders the operator-facing view

**Auth + Credits**
- Built enough for the current prototype; public rollout still needs a broader release plan

---

## Summary

| | User | Company |
|---|---|---|
| Installs | `opengpu` CLI | Control plane, dashboard, infra |
| Runs always | Node agent (background) | Control plane API |
| Runs per job | Worker subprocess | — |
| Maintains | Nothing — just keep connected | All shared infrastructure |
| Pays for | Nothing (contributor) | Hosting, bandwidth, build infra |

---

## What Needs to Be Open Sourced (for Public Trust)

Users contribute their hardware and run code on their machines. For them to trust the project, they must be able to verify what that code actually does.

### Must be open source

| Component | Why |
|---|---|
| **CLI** (`opengpu`) | Runs on the user's machine. They must be able to audit every command — connect, disconnect, what data is sent. |
| **Node Agent** | Runs persistently in the background. Users need to verify it isn't mining, exfiltrating data, or abusing resources beyond what they agreed to. |
| **Worker (M-series)** | Executes jobs on the user's machine. Must be auditable to confirm it only runs inference and nothing else. |
| **Install script** | The first thing a user runs. A closed install script is a red flag — must be readable before execution. |
| **Protobuf / shared contracts** | Defines exactly what data flows between nodes and the control plane. Transparency here builds protocol trust. |

### Should stay transparent

| Component | Why |
|---|---|
| **Control plane** | Now lives in the private operator repo. The protocol and message shapes should stay documented publicly, but the service implementation can remain closed. |
| **Heartbeat + routing logic** | Nodes want to know how they're scored and selected — opaque routing creates suspicion of favoritism or hidden costs. |

### Can stay closed (acceptable)

| Component | Why |
|---|---|
| **Dashboard** | UI only, but the operator-facing implementation now lives in the private repo. |
| **Credits / billing logic** | Business-sensitive. Kept in the private repo while the accounting rules remain publicly documented. |
| **Auth service** | Internal token issuance. The protocol it enforces should still be documented. |
| **Deploy infra / CI secrets** | Never needs to be public. |

### The rule of thumb

> **Anything that runs on a user's machine must be open source. Anything that touches user data should be open source. Everything else is your call.**

### Recommended licensing

**Use Apache 2.0 for the user-facing and node-executed components** (CLI, agent, workers, contracts, install script).

Why Apache 2.0 over MIT:
- **Patent grant** — MIT has none. Apache 2.0 explicitly grants users a patent license, which matters when acceleration code touches patented territory.
- **Enterprise-friendly** — companies running nodes (the target contributors) prefer Apache 2.0 because their legal teams have pre-approved it. It's the standard for infrastructure projects.
- **Ecosystem alignment** — Kubernetes, TensorFlow, Tokio (the async runtime used here), and most serious Rust infrastructure use Apache 2.0 or dual Apache-2.0/MIT.

For the **control plane**: keep it proprietary in the private operator repo. That keeps the trust-sensitive orchestration layer private while the contributor-facing code stays auditable under Apache 2.0.

| Component | Recommended License | Rationale |
|---|---|---|
| CLI | Apache 2.0 | Runs on user machines, must be auditable |
| Node Agent | Apache 2.0 | Runs on user machines, must be auditable |
| Workers (M-series) | Apache 2.0 | Runs jobs on user hardware |
| Install script | Apache 2.0 | First thing a user runs |
| Protobuf / contracts | Apache 2.0 | Defines the protocol |
| Control plane | Proprietary | Private operator repo |
| Dashboard | Proprietary | Private operator repo |
| Credits / auth | Proprietary | Private operator repo |

### Practical setup

1. Keep the public repo root `LICENSE` file for the Apache 2.0 licensed parts of the repo.
2. Keep a separate private license notice in the operator repo.
3. Optionally add an SPDX header to each source file for machine-readable license scanning:

```rust
// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 MundusX, Inc.
```

SPDX headers are not required but make enterprise users and legal audits significantly easier, and are read automatically by GitHub, FOSSA, and OSS compliance tools.

### The rule of thumb

> **Anything that runs on a user's machine must be open source. Anything that touches user data should be open source. Everything else is your call.**
