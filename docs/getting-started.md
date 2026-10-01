# Install a MundusX OpenGPU contributor

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
The guides describe the 0.2.19 release bundle. Its CLI `--version` still reports
0.2.16 because package metadata was not bumped.

Installation is complete only after setup, startup, and readiness checks succeed.
A downloaded executable or saved model selection does not prove the node is
connected, warm, or admitted for image/video jobs.
