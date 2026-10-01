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

## Current-stage reporting

Runtime startup has one activity reporter per runtime. Docker layer downloads and
extraction retain their layer IDs and reported byte counts. vLLM reports model
file download, checkpoint loading, GPU graph compilation, warmup and endpoint
health separately when the runtime emits these events. MLX and llama.cpp report
recognized loading and warmup stages too. Unknown work remains labelled as a wait;
it is never presented as a measured percentage or confirmed completion.

Background startup stops its initial waiting timer when agent progress arrives.
The elapsed reminder names the last observed stage and says how long it has been
since progress changed. It does not prove that new bytes or work are completing.
Update downloads track the current asset and byte counts. Media helper activity
tracks its current dependency, file checksum/download or generation step.
