# MundusX Contributor Architecture

For the current repo layout and review rules, see
[docs/repo-standards.md](docs/repo-standards.md).

## Core principle

Contributor machines advertise measurable capabilities. Planned tasks declare
requirements. The control plane selects an eligible contributor, and the local
Node Agent makes the final admission decision before execution.

## Major Components

### CLI

- Separately installable binary
- Handles bootstrap, authentication, contribution setup, node control, status,
  logs, and updates
- Does not schedule or execute jobs

### Control Plane

- Registry of nodes
- Planner and job graph
- Capability-aware scheduler
- Task queue, reservations, and leases
- Result collection and verification
- Retry and fallback policy
- Policy engine
- Credits ledger

### Node Agent

- Runs locally on each machine
- Probes and reports hardware, runtime, models, health, capability, and
  availability
- Enforces safety, thermals, and user-first limits
- Polls for reserved tasks and performs a final local admission check
- Rejects incompatible work with a structured, retryable reason
- Launches workers under a time-bounded task lease

### Workers

- `M-series` backend for Apple Silicon
- `CUDA` backend for NVIDIA
- Execute dynamic task roles such as expert, reducer, or synthesizer when the
  machine satisfies the task requirements

Expert, reducer, and synthesizer are job roles, not permanent node types.

## Contributor job flow

```text
install CLI + Node Agent
  -> create identity
  -> probe and qualify capabilities
  -> register
  -> heartbeat current capacity
  -> poll reserved task
  -> run local admission checks
  -> claim lease
  -> execute worker
  -> report result
```

If admission fails or the node disappears, the control plane releases or
expires the lease and reassigns the task. Intermediate graph results are
retained centrally so a failed reducer or synthesizer step can be retried
without rerunning successful expert tasks.

### Dashboard

- Live network view
- Node health
- Job history
- Credits and usage
