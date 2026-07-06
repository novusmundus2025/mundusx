# vLLM backend

MundusX treats `vllm` as an explicit Linux NVIDIA backend. It is not selected by
`auto`, and it is not part of the Windows contributor path.

## Current contract

- Windows contributors keep using the signed `llama.cpp` CUDA runtime bundle.
- `opengpu start` with `backendPreference: auto` still detects Apple Silicon or
  CUDA only.
- `opengpu start --backend vllm` is accepted as an explicit preference, but the
  node agent will not advertise itself as ready until a vLLM runtime adapter is
  installed.
- `opengpu doctor --json` includes a `vllm` diagnostic block with OS, Python,
  vLLM module, NVIDIA driver visibility, and readiness details.

## Ubuntu target

The intended first vLLM target is Ubuntu/Linux with:

- NVIDIA GPU visible through `nvidia-smi`
- Python available as `python3`
- importable `vllm` module
- a future MundusX vLLM adapter that speaks the worker contract and returns
  output, error, model, runtime, backend, worker ID, and node ID metadata

Until that adapter exists, vLLM nodes are allowed to configure the backend for
diagnostics but must remain unavailable for scheduler claims.

## Why this is separated from Windows

Windows stability is the release priority for the current contributor CLI. vLLM
is primarily a Linux serving stack, so Windows must not be forced to install
Python, CUDA developer tooling, or vLLM packages just to run the normal
`opengpu start` path.
