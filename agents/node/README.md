# Node Agent

Rust node daemon that runs on contributing machines and reports availability, health, and capability.

The health probe supports the current Mac/BLAS path and the first Windows NVIDIA CUDA bring-up path. CUDA diagnostics use `nvidia-smi` to report device availability, driver/runtime readiness, device name, VRAM, and whether a 4 GB-class GPU should use the low-VRAM profile.

See [docs/node-agent.md](/Users/DBATALL/Documents/mundusx/docs/node-agent.md) for the current prototype commands and local state files.
