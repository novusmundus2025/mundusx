# Agent / Worker Contract

This page defines the interface the CLI, node agent, and worker should share when the next phase starts.

## Goal

Keep the CLI focused on user setup and machine readiness, while the agent handles background presence and the worker handles the actual local compute.

## Roles

- `CLI`: user-facing setup, start, status, and exit
- `Agent`: background service on the provider machine
- `Worker`: short-lived process that performs a job locally

## Registration

When the agent comes online, it should register with:

- node ID
- hostname
- public key fingerprint
- public key hex
- resolved backend
- contribution percent
- cap-aware capability advertisement, including physical VRAM, usable VRAM budget, active model, compatibility, runtime mode, and readiness
- agent version

## Heartbeat

The agent should periodically send a heartbeat that includes:

- node ID
- backend
- agent state
- available memory
- available GPU percent
- contribution percent
- worker health snapshot
- cap-aware capability advertisement
- scheduler capability profile inside `worker_health.capabilities`
- timestamp

The control plane uses that heartbeat to decide whether the node is:

- ready
- busy
- paused
- offline

## Worker Launch

When a job arrives, the agent should launch a worker locally with:

- job ID
- node ID
- backend
- prompt or task payload
- optional model name
- optional system prompt
- optional max token count
- optional temperature
- optional top-p
- optional seed

The worker should return:

- job ID
- worker ID
- status
- output text
- resolved backend
- node ID
- model name when known
- runtime mode when known
- optional error text
 
When running on Mac `M`, the worker uses the cached GGUF model with `llama.cpp` in single-turn batch mode via `llama-cli --device BLAS`.
It should keep the default load modest so the contributor machine stays responsive, and it should return only the final answer text instead of the full runtime transcript.

## Job Queue And Completion

The current prototype adds one small control-plane queue:

1. A client, SDK, or dashboard submits a job to `POST /v1/jobs`.
2. The control plane stores the job as `queued`.
3. The agent asks `GET /v1/jobs/next?node_id=...` for work.
4. If a queued job matches the node backend, the control plane marks it `assigned`.
5. The agent sends a busy heartbeat and launches the worker locally.
6. For a live-stream-enabled job, the agent posts signed, monotonically sequenced text deltas to `POST /v1/jobs/delta`. Deltas are transient delivery messages, not jobs, durable events, or credit entries.
7. The authoritative worker result is posted once to `POST /v1/jobs/complete` with output, error, duration, model/runtime, backend, worker ID, and node ID metadata.
8. The control plane marks the job `completed` or `failed` and settles completion-based credits once.
9. The agent sends a ready or paused heartbeat after completion so the control plane can keep scheduling decisions current.

If the worker subprocess fails before returning a normal result, the agent still posts a failed completion for the claimed job. That failure includes the node ID, selected backend, model when known, runtime mode when known, duration, and actionable error text so the control plane can expose the failed lifecycle cleanly.

## Local-first contributor requests

The node agent is the single admission authority for both contributor-local work and control-plane work on that machine. It owns one shared slot pool, exposes an authenticated loopback API at `127.0.0.1:11435`, and never binds the API to a non-loopback address.

`opengpu run` defaults to `--routing local-first`: it asks the local agent first when the execution mode is `single`, then silently falls back to the normal control-plane queue if the local model, backend, slot, token, runtime, or agent is unavailable. Local rejections carry a stable code such as `LOCAL_MODEL_UNSUITABLE`, `LOCAL_CAPACITY_UNAVAILABLE`, or `LOCAL_RUNTIME_FAILURE` plus an explicit fallback permission. A successful network fallback keeps this diagnostic metadata in JSON output but does not present the local rejection as a user-facing error. If both permitted routes fail, the terminal error identifies both causes. `--routing local-only` forbids fallback, and `--routing network-only` skips the local attempt. Decomposed work remains control-plane routed.

Local availability is necessary but not sufficient. Before starting ordinary `local-first` work, the agent applies a conservative suitability gate using the active model's reported or name-derived parameter tier plus request shape and output budget. Small models (up to 4B parameters) may handle lightweight chat, translation, extraction, classification, rewriting, and short summaries, but defer substantial code generation, complex reasoning or synthesis, long-context work, and large outputs to the control-plane planner. Explicit `--routing local-only` is the contributor's override and bypasses this suitability gate.

The node-owned slot pool, not the control plane, authorizes local execution. After reserving a local slot, the agent starts the worker without waiting for control-plane permission. In parallel it reports occupancy through the existing signed, short-lived lease API using only node, request, model, slot count, and expiry metadata; the prompt and output never enter that report. A granted occupancy record is renewed during execution and released afterward. A declined, unauthenticated, misconfigured, or unreachable report never stops the owner's local work and never produces contributor credits. Network jobs must acquire the same node-owned slot pool, so stale control-plane state cannot oversubscribe the machine.

The loopback bearer token is stored at `~/.opengpu/local-agent-token`. Setting `OPENGPU_LOCAL_FIRST_ENABLED=false` disables the loopback service for rollback; the CLI then follows its selected fallback policy.

When occupancy reporting encounters a transport failure, the result is marked `local-offline`; the worker is never interrupted. The reporter retries every five seconds and synchronizes the active occupancy if connectivity returns. If a small local model initially defers a complex request but the subsequent control-plane route fails at the transport layer, `local-first` performs one forced local best-effort retry. It does not retry local runtime failures or remote application-level failures, preventing duplicate execution. Offline work cannot award credits or create a normal control-plane job record.

## Output Path

1. Client sends a request to the control plane.
2. Control plane queues the job.
3. Agent claims the job when it is ready.
4. Agent launches the worker locally.
5. Worker runs on `M` series using the local Mac runtime.
6. Worker returns output to the agent.
7. Agent forwards the result upstream.

## What The CLI Needs To Be Ready For

- show the resolved backend cleanly
- expose the device identity and public key fingerprint
- preserve contribution percent and pause state
- keep room for local agent readiness checks
- avoid mixing demo node inventory into the main status view

## Status Of This Contract

The data shapes are now defined in:

- `apps/cli/src/types.rs`
- `packages/proto/schema/opengpu.proto`

The transport and service implementations are partially in place, including:

- agent registration and heartbeat transport to the control plane with device signatures
- control-plane job queue submission and claim flow
- local worker launch from the agent

The device keypair is now the node identity layer. The control plane verifies the node's signed requests instead of requiring a separate contributor login for the machine itself. The hostname and `identityTrustPath` are part of the signed contributor identity so the operator can see which physical machine is represented and whether it is using Keychain or the local encrypted fallback without relying on an unauthenticated label.

The health check command now verifies the local Mac runner without starting a full job, and it also reports whether the current contribution cap is allowed by the Mac power state. The agent now converts that policy into paused heartbeats so the control plane will not assign work when the Mac should stay quiet. The same heartbeat also carries the worker health snapshot so the control plane can show readiness before it assigns work. The worker launch payload now carries the execution profile too, so the control plane can hand the worker a real system prompt, max token count, temperature, top-p, and seed instead of only a bare prompt string.

Registration and heartbeat payloads also carry a top-level `capabilities` object. That object is the readiness gate for the selected contribution cap, physical CUDA VRAM when known, cap-applied usable VRAM, runtime mode, active model metadata, model compatibility, and whether the node is ready for jobs. If no active model is configured, the worker health is degraded, policy blocks execution, or the active model is rejected, the node must advertise `ready_for_jobs=false` with a reason instead of appearing schedulable.

Heartbeats also carry `worker_health.capabilities`, which is the first-class scheduler profile used to group and rank nodes after readiness is known. It includes:

- `models`: every model the node can serve under the current contribution cap. Schema v4 records each model's `active`, `warm`, context/output limits, capacity class, roles, and task capabilities.
- `max_context_tokens`: the largest conservative context budget across the servable inventory
- `total_vram_mb` and `available_vram_mb`: physical and contribution-capped GPU budget when known
- `max_parallel_jobs`: worker concurrency budget, never higher than the memory-derived or runtime-reported safety ceiling; a contributor setting may only lower it
- `current_load_percent`: current load derived from available GPU percentage
- `roles`: scheduler roles such as `chat`, `coding`, `batch`, `reducer`, `vision`, `embedding`, or `tool_use`
- `skill_tags`: normalized matching tags such as `backend:cuda` and `runtime:cuda`

The scheduler should treat the top-level `capabilities.ready_for_jobs` as the eligibility gate, then choose a node and a specific model together. It first filters for eligibility and current availability, then ranks the remaining choices by task appropriateness, quality, latency, load, reliability, and resource fit. Model size is one fit signal, not an automatic preference for the smallest model. A claimed job carries the selected model name, so the worker executes the model the scheduler evaluated. Legacy agents without a schema-v4 model inventory continue through the node-wide compatibility path.

## SpeakAI structured completion gate

The node worker recognizes SpeakAI jobs from `mode: "speakai"`. The control plane
forwards that mode and the user's message without adding SpeakAI schema or retry
instructions. MundusX owns the complete SpeakAI prompt and schema. These jobs use a
stricter completion pipeline than ordinary chat:

- OpenAI-compatible llama-server, vLLM, and contributed-cluster requests include a
  strict `json_schema` response format. Direct llama.cpp CLI execution uses an
  equivalent JSON grammar. Runtimes without grammar support still pass through the
  same validator.
- The worker parses and validates the generated object before reporting a completed
  job. Malformed JSON, missing required content, unsupported speech acts, and reply
  counts other than three are never returned as completed Chat output.
- Validation failures receive at most two internal retries (three total attempts).
  Retry prompts include the validation reason, use a deterministic zero temperature,
  and increment the request seed.
- Successful output is serialized as one compact JSON object. Field whitespace and
  speech-act casing are normalized; unknown fields are removed; non-question
  `questionType` values are removed; missing/unsupported question types normalize to
  `other`; and reply `strategy`/`purpose` values are derived from the canonical
  speech-act contract. Reply `text` and English `meaning` must remain non-empty.
- When a client did not request a model explicitly, SpeakAI prefers an installed model
  advertising `speakai`/`structured_output` capability. Reviewed Qwen, Phi-4, and
  Gemma manifests are treated as structured-output-capable fallbacks. Explicit model
  requests are never silently replaced.

Greeting reply purposes are normalized from this owned contract to
`RETURN_GREETING`, `START_CONVERSATION`, and `WARM_VARIATION`, even when a model
generates conflicting labels. Production promotion or deployment is outside this
worker contract and requires separate approval.
