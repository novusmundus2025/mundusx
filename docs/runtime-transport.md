# Runtime And Transport Map

This page explains which OpenGPU components are local processes, which ones are HTTP servers, and which ones only speak HTTP as clients.

## Current Prototype

| Component | Runs As | Uses HTTP? | Notes |
|---|---|---|---|
| `opengpu` CLI | Local user command | Client only | Runs on the user machine. It reads local config, starts onboarding, and can talk to the control plane when needed. |
| Node agent | Local background process | Client only | Runs on provider machines. It registers, heartbeats, and polls for jobs over HTTP. |
| Worker | Short-lived local subprocess | No server | Runs the actual per-job compute locally. It is launched by the agent. |
| Control plane | HTTP server | Yes | The source of truth for node registration, heartbeats, job queue, and job completion. In the prototype it listens on `http://127.0.0.1:8787` and serves a browser-friendly root page at `/`. |
| Dashboard | Planned web app | Yes, later | Not built yet. Will read control-plane state through HTTP APIs. |
| Install script | Shell bootstrapper | Client only | Downloads the CLI binary and installs it. It is not a server. |

## What Talks To What

- The CLI can talk to the control plane for setup and status.
- The node agent talks to the control plane for registration, heartbeat, job claim, and completion.
- The control plane does not run jobs itself.
- The worker does not serve HTTP.
- The dashboard, when built, will query the control plane.

## Current Local Prototype URLs

- Control plane: `http://127.0.0.1:8787`
- Browser health page: `http://127.0.0.1:8787/`
- CLI config can point the agent at that URL for local smoke tests.

## Rule Of Thumb

- **Local process** means it runs on the machine as a binary or subprocess.
- **HTTP server** means it listens for inbound requests.
- **HTTP client** means it sends requests but does not listen.
- **Server** in product language usually means the control plane or dashboard backend, not the worker.

## How To Update This Page

- If a component starts listening on a port, update the `Runs As` and `Uses HTTP?` columns.
- If a component stops talking to the control plane, update the `What Talks To What` section.
- If the local prototype port changes, update the URL list.
