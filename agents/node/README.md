# Node Agent

Rust node daemon that runs on contributing machines and reports availability, health, and capability.

The health probe supports the current Mac/BLAS path and the first Windows NVIDIA CUDA bring-up path. CUDA diagnostics use `nvidia-smi` to report device availability, driver/runtime readiness, device name, VRAM, and whether a 4 GB-class GPU should use the low-VRAM profile.

Enterprise Windows hosts can pin trusted runtime executables in the non-secret config file `~/.opengpu/trusted-runtime-paths.json` so worker launch and diagnostics do not depend on PATH order:

```json
{
  "llama_cli": {
    "path": "C:\\Program Files\\MundusX\\llama-cli.exe",
    "sha256": "expected lowercase sha256"
  },
  "nvidia_smi": {
    "path": "C:\\Windows\\System32\\nvidia-smi.exe"
  }
}
```

When this file is present, paths must be absolute. Missing executables, hash changes, and relative paths are reported as missing or untrusted runtime diagnostics instead of falling back to PATH lookup.

See [docs/node-agent.md](/Users/DBATALL/Documents/mundusx/docs/node-agent.md) for the current prototype commands and local state files.
