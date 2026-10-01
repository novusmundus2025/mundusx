# CLI and agent progress coverage

The cross-platform progress update covers Windows, Linux, and Apple Silicon macOS.
Progress is written to stderr so JSON stdout remains usable. Activity messages
show the current operation and elapsed time every ten seconds, without inventing
percentages. Completion and failure are reported by the operation's existing result.

| Operation | Feedback |
| --- | --- |
| POSIX bootstrap | Initial message, curl download bar, timed network/GPU/Docker activity |
| Windows bootstrap | Byte/percentage download progress, network-wait activity, extraction indicator |
| CLI self-update | Asset name, received bytes and percentage when total is known; timed activity while waiting |
| GGUF model download/import | curl bar for downloads; elapsed activity for copying and checksum verification |
| Python/MLX dependencies and virtual environment | Step label and elapsed activity |
| MLX/vLLM model prefetch | Native downloader output and elapsed activity |
| External engine verification | Verification step and elapsed activity |
| Background agent startup | Immediate wait message, timed activity, relayed runtime progress, confirmed worker health |
| Managed vLLM | Container preparation activity, recognized download/shard counts, endpoint health check |
| Managed MLX and llama.cpp | Runtime preparation activity; MLX warmup has a distinct activity label |
| Media setup and verification | Helper stage messages, byte download progress, timed activity |
| Media generation | Existing elapsed generation events; no fabricated overall percentage |
| Separate MundusX connector | Server startup and browser-approval activity; existing Hermes installation activity retained |
| Inference jobs | Existing streamed output / graph-chunk progress retained |

A configuration and active model alone no longer imply readyForJobs. Readiness
requires a node-agent report. A warm process is not guaranteed by a download bar.
Existing subprocess timeouts and cancellation behavior remain in force.

A percentage is only shown when byte totals or shard counts are available. Model
compilation, warmup and unknown-size network waits show elapsed activity instead.
