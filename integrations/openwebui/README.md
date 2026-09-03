# MundusX for OpenWebUI

This integration keeps OpenWebUI as a replaceable chat interface. Sessions,
memory, policy, tools, and future delegation remain owned by the local MundusX
agent runtime.

1. Start the OpenGPU local model runtime.
2. Start `mundusx-agent-server` on the default loopback address.
3. Import `mundusx_pipe.py` as an OpenWebUI Pipe Function.
4. Select **MundusX Agent** in OpenWebUI.

When OpenWebUI runs in Docker, the default Pipe URL uses
`host.docker.internal:11436`. For a native OpenWebUI installation, set
`MUNDUSX_AGENT_URL` to `http://127.0.0.1:11436`.

Set the same `MUNDUSX_AGENT_API_KEY` value for the server environment and the
Pipe Valve when OpenWebUI is not the only local user. The server refuses
non-loopback binds; remote access should go through an authenticated proxy.

The Pipe uses a complete response while the generic OpenAI-compatible endpoint
also accepts `stream=true`. Durable structured events are available at
`GET /v1/sessions/{session_id}/events`; the adapter never becomes a second
agent loop.
