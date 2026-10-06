# Install a MundusX OpenGPU contributor

**GB10/GX10 contributors:** start SparkRun and verify the model first, then connect MundusX to the existing runtime. Use the [SparkRun installation guide and recipe files](https://mundusx.github.io/mundusx/install-sparkrun/).

Choose the guide for the computer that will contribute:

| Machine | Step-by-step guide |
| --- | --- |
| Windows x86_64, CUDA or Vulkan LLM runtime | [Windows installation](install-windows.md) |
| Linux x86_64 or ARM64, including GB10/GX10 | [Linux installation](install-linux.md) |
| Apple Silicon Mac with MLX | [macOS and MLX installation](install-macos-mlx.md) |

Then follow [the setup choices and connection checklist](contributor-setup.md).
For eligible hardware, continue to [image and video setup](install-media.md).

These guides cover compute contribution through `opengpu`, not the separate
`mundusx connect` development-agent Chat pairing workflow. Public contributor
setup does not require Rust, Cargo, a GitHub token, or an account password.

Downloads come from the [MundusX production release](https://github.com/mundusx/releases/releases/tag/opengpu-prod).
These guides describe release **0.2.23**. Check your installed CLI with
`opengpu --version`; it should report `opengpu 0.2.23` for this release.

Choose a managed standalone model or reuse a direct local engine, including
Ollama, LM Studio, vLLM, or llama.cpp. See [Inference connections](inference-connections.md)
for endpoint/model selection. NVIDIA PAIR currently supports endpoint validation
only; it cannot start contribution.

Installation is complete only after setup, startup, and readiness checks succeed.
A downloaded executable or saved model selection does not prove the node is
connected, warm, or admitted for image/video jobs.
