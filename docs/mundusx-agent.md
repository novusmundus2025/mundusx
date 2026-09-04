# MundusX local agent

MundusX is the personal, local-first agent surface. OpenGPU remains the
contributor surface, and the hosted Control Plane remains the optional network
coordination surface. The local agent does not require the Control Plane.

## Commands

```text
mundusx run "inspect this repository"
mundusx agent run "inspect this repository"
mundusx agent resume <session-id> "continue and explain the failing test"
mundusx agent sessions
mundusx agent cancel <session-id>
mundusx connect --workspace .
mundusx model list
mundusx model use <model>
mundusx model add <model>
mundusx model remove <model>
```

`mundusx model` currently delegates to the proven OpenGPU model manager. The
equivalent `opengpu model` commands remain supported as compatibility aliases.
Contributor lifecycle commands remain under `opengpu`.

## MundusX Chat

`mundusx connect` makes an outbound HTTPS connection to `chat.mundusx.ai`, so
the hosted chat can use this same local agent without exposing the loopback
server to the internet. Create a connection token in Chat under Account > MCP
connections, then provide it without placing it in shell history:

```powershell
$env:MUNDUSX_CHAT_TOKEN = "<connection-token>"
mundusx connect --workspace .
```

The older top-level `mundusx run`, `resume`, `sessions`, and `cancel` forms remain
compatible. New documentation and integrations should use the `mundusx agent`
namespace so contributor and model management commands remain unambiguous.

Deep Agents is an optional runtime plugin. Set `MUNDUSX_DEEPAGENTS_BIN` to the
installed `mundusx-deepagents-runtime` executable before starting
`mundusx connect`. The connector advertises `deepagents` only after verifying
that executable exists; otherwise all `auto` tasks safely use the native agent.

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
