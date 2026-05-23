# Control Plane

The control plane is the source of truth for node registration and heartbeat state.

For the current prototype, it listens on:

```text
http://127.0.0.1:8787
```

The root URL (`/`) now returns a small HTML health dashboard for browser checks.
It includes per-node rows so you can see backend, power, battery, policy state, and policy reason directly in the browser.

## Prototype Endpoints

- `GET /` - browser-friendly health and status page
- `GET /health` - health check
- `GET /v1/status` - return a snapshot of the current registry
- `GET /v1/nodes` - return the live node list
- `GET /v1/jobs` - return all known jobs
- `GET /v1/jobs/next?node_id=...` - claim the next queued job for a node
- `POST /v1/register` - register an agent
- `POST /v1/heartbeat` - update a node heartbeat
- `POST /v1/jobs` - submit a job request
- `POST /v1/jobs/complete` - complete a claimed job

## Submitting A Job

A client, SDK, or dashboard sends the request to the control plane:

```bash
curl -X POST http://127.0.0.1:8787/v1/jobs \
  -H 'Content-Type: application/json' \
  -d '{
    "request_id": "job-001",
    "prompt": "Summarize this paragraph",
    "preferred_backend": "auto",
    "model": null
  }'
```

Then an active node agent claims it with:

```text
GET /v1/jobs/next?node_id=...
```

The agent launches the worker locally, then posts the result to:

```text
POST /v1/jobs/complete
```

## Current Storage Model

The prototype keeps its state in a local JSON file, and mirrors the same events into Supabase over HTTP when the Supabase env is configured:

- `~/.opengpu-control-plane/state.json`

or, if configured:

- `OPENGPU_CONTROL_PLANE_HOME/state.json`
- `OPENGPU_HOME/state.json`

If Supabase is not configured, the local JSON state remains the fallback.
The applied SQL schema lives at [supabase/schema.sql](/Users/DBATALL/Documents/aigrid/supabase/schema.sql).

## What The State Contains

- node ID
- public key fingerprint
- backend
- contribution percent
- agent version
- agent state
- available memory
- available GPU percent
- power source
- on-battery state
- battery percent
- policy allowed / blocked
- policy reason
- last updated timestamp
- job ID
- job request ID
- queued / assigned / completed / failed job state
- assigned node
- worker result and error details

## Current Behavior

- registration inserts or updates a node record
- heartbeat updates the node record, refreshes the timestamp, and stores the Mac policy fields
- `POST /v1/jobs` queues a job request in local JSON state
- `GET /v1/jobs/next?node_id=...` lets a node claim the next queued job
- nodes with `policyAllowed: false` are not eligible for job claims
- `POST /v1/jobs/complete` stores the worker result and marks the job complete or failed
- the control plane mirrors registration, heartbeat, claim, job, and completion events into Supabase when configured
- status returns a snapshot with:
  - total nodes
  - online count
  - paused count
  - policy blocked count
  - stopped count
  - queued jobs
  - assigned jobs
  - completed jobs
  - failed jobs

## Browser Dashboard

The root page (`/`) is a quick operator view, not a full dashboard. It shows:

- summary counts
- policy-blocked count
- a per-node table with:
  - node ID
  - backend
  - node state
  - power source
  - battery state
  - policy allowed / blocked
  - policy reason
  - last updated timestamp

## What Comes Next

- routing policy
- auth
- durable storage
