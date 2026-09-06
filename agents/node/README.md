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

## Building From Source On Windows (Dev Machines)

`cargo build -p opengpu -p opengpu-node-agent --release` only builds the two Rust binaries. It does not fetch or pin the CUDA llama.cpp runtime the way `install.ps1` does, so a fresh dev machine will report `llamaCliAvailable: no` and `policyAllowed: no` from `opengpu-node-agent health` until `llama-cli.exe` is installed and pinned.

Run `scripts/windows-dev-llama-runtime.ps1` after (or instead of) a manual `cargo build` to automate that setup:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\windows-dev-llama-runtime.ps1
```

It builds both binaries, detects the NVIDIA driver's supported CUDA version, downloads the matching `llama.cpp` Windows CUDA release and `cudart` runtime zip from [ggml-org/llama.cpp releases](https://github.com/ggml-org/llama.cpp/releases), extracts `llama-cli.exe` into `<OPENGPU_HOME>\runtimes\llama`, pins it in `trusted-runtime-paths.json`, and runs `opengpu-node-agent health` to confirm. Pass `-SkipBuild` to only refresh the runtime, `-CudaVersion 12.4` (or `13.3`) to override auto-detection, or `-Tag b9856` to pin a specific `llama.cpp` release instead of latest.

See [docs/node-agent.md](/Users/DBATALL/Documents/mundusx/docs/node-agent.md) for the current prototype commands and local state files.
