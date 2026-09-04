# MundusX local agent

MundusX is the personal, local-first agent surface. OpenGPU remains the
contributor surface, and the hosted Control Plane remains the optional network
coordination surface. The local agent does not require the Control Plane.

## Commands

```text
mundusx run "inspect this repository"
mundusx agent install hermes
mundusx agent use hermes
mundusx agent status
mundusx agent use native
mundusx agent use none
mundusx resume <session-id> "continue and explain the failing test"
mundusx sessions
mundusx cancel <session-id>
mundusx connect --workspace .
mundusx model list
mundusx model use <model>
mundusx model add <model>
mundusx model remove <model>
mundusx contributor start
mundusx contributor stop
mundusx contributor status
mundusx contributor cap 50
mundusx contributor logs
```

`mundusx model` currently delegates to the proven OpenGPU model manager. The
equivalent `opengpu model` commands remain supported as compatibility aliases.
`mundusx contributor` delegates to the proven OpenGPU contributor
implementation. The equivalent `opengpu` commands remain supported as
compatibility aliases.

## Hermes runtime

Hermes is an optional execution harness, not the MundusX control boundary.
Install it with `mundusx agent install hermes`; the verified installer pins an
upstream Hermes commit and checksum. The selected harness is stored per user in
`~/.mundusx/agent-config.json`. `--runtime hermes` remains available as a
one-command override. Set `MUNDUSX_HERMES_BIN` only when the executable is not
on `PATH`. MundusX maps its session IDs to Hermes session IDs locally and
resumes the corresponding Hermes conversation.

When the MundusX node is running with an active model, Hermes automatically
uses its authenticated loopback-only raw inference endpoint. If that endpoint
is offline, Hermes uses its configured cloud provider instead; run `hermes
setup` only when a cloud fallback is wanted. MundusX continues to own model
selection and contributed compute in either case.

Connected Chat devices advertise Hermes only after `hermes --version` succeeds.
Chat advertises the saved preference and Auto routing honors it; otherwise
tasks use the native runtime. A Hermes coding task receives `--yolo` only when the submitting user
explicitly enables mutations. Without that grant, Hermes retains its approval
gate and non-interactive writes fail closed. For strong isolation, configure a
Hermes Docker/SSH/Daytona terminal backend; an in-process tool allowlist is not
an OS security boundary.

For Chat Projects, start the connector at the parent directory that contains
the project slugs. Chat sends only a lowercase project slug; the connector
rejects absolute paths and traversal, canonicalizes the target, and runs Hermes
with that directory as its working-directory boundary. A missing project
directory is created only when the Chat task carries explicit mutation
authority.

```powershell
mundusx connect --workspace "$env:USERPROFILE\Documents\mundusx\projects"
```

## MundusX Chat

`mundusx connect` makes an outbound HTTPS connection to `chat.mundusx.ai`, so
the hosted chat can use this same local agent without exposing the loopback
server to the internet. Create a connection token in Chat under Account > MCP
connections, then provide it without placing it in shell history:

```powershell
$env:MUNDUSX_CHAT_TOKEN = "<connection-token>"
mundusx connect --workspace .
```

Chat falls back to the Control Plane when the connector is offline. The bridge
uploads privacy-filtered progress metadata and the final response; local tool
arguments, results, files, and transcripts stay on the machine. Remote URLs
must use HTTPS, and revoking the Chat token disconnects the agent.

## Runtime boundary

The `mundusx-agent-server` binds to `127.0.0.1:11436` by default and exposes an
OpenAI-compatible `/v1/chat/completions` endpoint. `mundusx run` starts it on
demand. Set `MUNDUSX_AGENT_API_KEY` when another local application needs to
connect. The server accepts a fixed workspace at startup; clients cannot change
that boundary in a request.

Sessions are append-only SQLite event streams in `~/.mundusx/agent.db` (or
`MUNDUSX_HOME`). Read-only inspection tools run without approval. File patches,
validation commands, and durable memory writes are denied unless the caller
explicitly enables mutations. CLI users do that with `--approve-mutations`.

Local skills are loaded from `<MUNDUSX_HOME>/skills/<skill>/SKILL.md` and selected
for a request by their name and description. Selected skill names, context
compaction, approvals, tool activity, and final responses are recorded as
session events.

Requests execute concurrently. A client that supplies a session id can cancel
between model/tool turns with `POST /v1/sessions/<session-id>/cancel`; cancellation
is persisted in that session's event stream.

Control Plane delegation is disabled by default. Set
`MUNDUSX_CONTROL_PLANE_URL` (and optionally `MUNDUSX_CONTROL_PLANE_TOKEN`) to
register the `task.delegate` tool. Delegation sends only the explicit prompt the
agent proposes, never the complete local transcript, and requires mutation
approval before any network request.

## Open WebUI

`integrations/openwebui/mundusx_pipe.py` is a thin adapter. It forwards chat
messages to the local service and keeps all agent policy, tools, memory, and
session state inside MundusX. When Open WebUI runs in Docker, the default URL is
`http://host.docker.internal:11436`.

## Architecture rule

Agent behavior belongs in provider-neutral packages (`agent-core`,
`agent-tools`, and `agent-skills`). CLI, Open WebUI, and future UIs are adapters.
OpenGPU contributor code and Control Plane scheduling must not become required
dependencies of the local agent loop.
