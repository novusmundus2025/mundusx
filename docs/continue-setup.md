# Continue setup

Merge the model entry from `examples/continue/config.yaml` into your local
`~/.continue/config.yaml` (Windows: `%USERPROFILE%\.continue\config.yaml`).
Keep any unrelated models and settings. Reload Continue if configuration changes
are not picked up, then start a new conversation.

This configuration uses the direct Chat-compatible API at
`https://chat.mundusx.ai/v1`, with the 131,072-token context limit. It does not route
through the Hermes task adapter. Continue owns local file tools and permissions.

For the currently deployed gateway, omit the explicit `tool_use` capability.
Continue 2.0.0 uses text-formatted tools for this custom model in that configuration,
allowing the existing text stream to arrive incrementally. Adding `tool_use`
switches to native tool calls, whose arguments are currently buffered by the
worker/gateway. `stream: true` alone does not change that behavior.

Verified on 2026-09-10: a direct request using text-formatted tool instructions
received 114 content chunks, beginning at 2.52 seconds and ending at 26.13 seconds.
This verifies the API transport; rendering and file-creation approval remain
Continue behavior. The example does not turn off approval or execute a tool itself.

Continue's built-in web search is separate from MundusX/Hermes web search. A search
service error should not be addressed by setting `MUNDUSX_WEB_SEARCH_URL` here.

Native tool-fragment streaming is a separate draft requiring coordinated worker
and gateway deployment. Do not enable it just by adding a capability to this file.
