# Requestor API

This page describes the public-facing request intake layer for subscribers, SDKs, and compatible clients.

The first compatibility endpoint is:

- `POST /v1/chat/completions`

It accepts a request shaped like an OpenAI / OneAPI chat completion request, then maps it into NovusX's internal job queue.

## What It Does Today

- Accepts `model`, `messages`, `temperature`, `top_p`, `max_tokens`, and `seed`.
- Converts chat messages into an internal prompt plus optional system prompt.
- Submits a queued job to the control plane.
- Returns an OpenAI-shaped `chat.completion` envelope with NovusX metadata.
- If `OPENGPU_OPERATOR_TOKEN` is configured, the route requires a matching bearer token just like the other operator-facing control-plane routes.

## Current Response Shape

The endpoint currently returns a queued response rather than a streamed final completion.

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

## Compatibility Notes

- This is the first requestor compatibility layer, not a full OpenAI clone yet.
- Streaming is not supported yet.
- The job still completes through the existing control-plane and worker pipeline.
- Contributors still claim work based on availability, backend compatibility, policy state, and worker health.

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

- streaming responses
- `/v1/responses` compatibility
- `GET /v1/models`
- requestor UI / SDK helpers
- richer retry and timeout policy
