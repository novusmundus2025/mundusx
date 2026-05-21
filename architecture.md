# OpenGPU Architecture

## Core Principle

Pick the best live node for a request, then execute the full request there.

## Major Components

### CLI

- Separately installable binary
- Handles bootstrap, auth, node control, status, logs, and updates

### Control Plane

- Registry of nodes
- Request router
- Policy engine
- Job queue
- Credits ledger

### Node Agent

- Runs locally on each machine
- Reports health, capability, and availability
- Enforces safety, thermals, and user-first limits

### Workers

- `M-series` backend for Apple Silicon
- `CUDA` backend for NVIDIA

### Dashboard

- Live network view
- Node health
- Job history
- Credits and usage

