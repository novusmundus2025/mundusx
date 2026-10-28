# macOS 0.2.54 manual release

This source snapshot includes the managed MLX native-tool runtime qualified on
the public Apple Silicon contributor. The runtime preserves the fifth tokenizer
result used by MLX versions with vision metadata and requires runtime capability
version 10 so an older warm process cannot silently retain the incompatible wrapper.

Qualification: 21 MLX runtime tests; live Chat Completions function call and tool
result round trip; streamed function call; stateless Responses function call and
tool result round trip through the public Railway inference API.

The CLI and worker are version 0.2.54. Builds and release uploads are manual;
GitHub Actions are not used. Existing nodes are not automatically restarted.
