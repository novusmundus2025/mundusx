# Requestor API

This page defines the public requestor-facing compatibility contract for subscribers, SDKs, and compatible clients.

The public repo owns the requestor API contract and examples. The operator repo can keep the implementation private as long as it continues to satisfy the behavior documented here.

## Compatibility Surface

The current requestor surface is:

- `POST /v1/chat/completions`
- `GET /v1/models`
- `stream: true` server-sent event responses for chat completions
- explicit retry, timeout, and idempotency rules for request submission

The requestor layer stays OpenAI-shaped where that helps client compatibility, but the control plane still maps accepted work into NovusX's internal queued-job pipeline.

## What It Does Today

- Accepts `model`, `messages`, `temperature`, `top_p`, `max_tokens`, and `seed`.
- Accepts `stream` when the client wants a `text/event-stream` response.
- Converts chat messages into an internal prompt plus optional system prompt.
- Submits the request to the control plane with explicit request metadata.
- Returns an OpenAI-shaped `chat.completion` envelope with NovusX metadata when `stream` is absent or `false`.
- Returns a `text/event-stream` response when `stream` is `true`.
- If `OPENGPU_OPERATOR_TOKEN` is configured, the route requires a matching bearer token just like the other operator-facing control-plane routes.
- It does **not** browse the internet by itself.
- It does **not** infer retrieval mode inside the worker.

## Current Response Shape

When `stream` is absent or `false`, the endpoint returns a queued response envelope immediately.

Example fields:

- `id`
- `object: "chat.completion"`
- `created`
- `model`
- `choices[0].message.role`
- `choices[0].message.content`
- `choices[0].finish_reason: "queued"`
- `opengpu.job_id`
- `opengpu.request_id`
- `opengpu.status`

## `GET /v1/models`

`GET /v1/models` returns the requestor-visible model catalog.

The response shape should stay compatible with a compact OpenAI-style models listing:

- `object: "list"`
- `data[]`
- `data[].id`
- `data[].object: "model"`
- `data[].owned_by`
- `data[].context_window`
- `data[].backend`
- `data[].capabilities`

The listing is requestor-facing, not a dump of every worker-local cache entry. Only models that the operator intends to expose to requestors should appear here.

Example response:

```json
{
  "object": "list",
  "data": [
    {
      "id": "HuggingFaceTB/SmolLM2-135M-Instruct",
      "object": "model",
      "owned_by": "mundusx",
      "context_window": 8192,
      "backend": ["m"],
      "capabilities": ["chat", "stream"]
    }
  ]
}
```

## Streaming Responses

When the request includes `"stream": true`, `POST /v1/chat/completions` should reply with `Content-Type: text/event-stream`.

The stream should stay line-oriented and predictable for thin SDK clients:

- emit a queue-acceptance event first so the client receives `job_id` and `request_id`
- emit delta events as output becomes available
- emit a terminal `done` event exactly once
- emit an `error` event before closing if the request fails after acceptance

Minimum event payload fields:

- `type`
- `request_id`
- `job_id`
- `created`
- `model`

Recommended event sequence:

1. `event: queued`
2. `event: delta`
3. `event: done`

Example:

```text
event: queued
data: {"type":"queued","request_id":"req_123","job_id":"job_123","created":1717000000,"model":"HuggingFaceTB/SmolLM2-135M-Instruct"}

event: delta
data: {"type":"delta","request_id":"req_123","job_id":"job_123","delta":"NovusX is a distributed GPU network."}

event: done
data: {"type":"done","request_id":"req_123","job_id":"job_123","finish_reason":"stop"}
```

## Retry, Timeout, And Idempotency

Request submission must be safe for normal client retries.

### Idempotency

- Clients may send `Idempotency-Key: <opaque-token>` on `POST /v1/chat/completions`.
- The control plane should treat the tuple of `(method, route, idempotency key)` as the deduplication key.
- Replays with the same idempotency key and the same effective request body should return the original accepted response instead of enqueueing duplicate jobs.
- Replays with the same idempotency key but a different body should fail with a client error.

### Timeout

- Clients may send `X-Request-Timeout-Ms` to declare the longest acceptable end-to-end wait for the request.
- If the header is absent, the requestor layer should apply a reasonable operator default.
- Timeouts shorter than the queue-acceptance path should fail fast rather than enqueueing work that the caller has already abandoned.
- A timed-out accepted request should preserve its job record so operators can audit what happened.

### Retry

- Transport-level failures before acceptance may be retried by the client with the same idempotency key.
- Once the requestor layer has accepted the job and returned `job_id`, retries should be treated as status lookups of the same logical request, not new work.
- Backoff should be exponential with jitter for automated clients.

## Compatibility Notes

- This is a compatibility layer, not a byte-for-byte OpenAI clone.
- The job still completes through the existing control-plane and worker pipeline.
- Contributors still claim work based on availability, backend compatibility, policy state, and worker health.
- A later gateway layer may attach `knowledge_mode` to decide between `model_only`, `retrieval`, and `web_search`.
- If that happens, the worker should receive only the resulting context, not the routing decision itself.

## Example

```bash
curl -X POST http://127.0.0.1:8787/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "HuggingFaceTB/SmolLM2-135M-Instruct",
    "messages": [
      {"role": "system", "content": "You are concise."},
      {"role": "user", "content": "Summarize NovusX in one sentence."}
    ],
    "temperature": 0.2,
    "top_p": 0.9,
    "max_tokens": 128,
    "seed": 42
  }'
```

The control plane stores the request as a job and returns a queued NovusX response envelope immediately.

## What Comes Next

- request policy metadata for `model_only`, `retrieval`, and `web_search`
- `/v1/responses` compatibility
- requestor UI / SDK helpers
