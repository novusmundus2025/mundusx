# Workload setup

Interactive `opengpu install` uses checkboxes for LLM, Qwen images, and Wan
videos. Move with the arrow keys or Tab, toggle with Space, select available
operations with A, and continue with Enter. Editing operations remain visibly
unavailable. Existing scripted `--workloads` arguments remain compatible.

Setup discovers model endpoints, preserves the saved contribution cap, and
skips local model provisioning when a cluster is selected. A saved custom cap
has its own selected row. Cancelling the cap menu stops before provisioning.

For media, setup probes the saved ComfyUI endpoint or localhost:8188. A detected
endpoint is offered first, but generation still requires model/workflow and
output verification. Automatic managed setup is offered only on Linux ARM64
with a detected GB10. Other machines can connect to existing ComfyUI.

Setup displays each selected profile's plan before installation. Existing
models are checked by size and SHA-256 and reused. Required free space counts
missing or invalid models plus runtime headroom. Downloads display bytes and
a progress bar in an interactive terminal; redirected output retains JSON
events. Generation displays elapsed time because polling does not provide a
reliable completion percentage. Runtime build and pull output remains visible.

The summary distinguishes saved configuration, verification, and admission.
Managed ComfyUI starts on demand, so an idle managed installation does not need
to answer on port 8188. Selecting an operation does not start a network worker.

## Current boundaries

- Images and videos support queued generation with the updated media server and worker.
- Media network serving requires an admitted contributor through the existing LLM admission path.
- Existing HTTP model servers can be discovered or specified explicitly;
  Sparkrun recipe provisioning is not implemented.
- Full GPU installation and generation must be validated separately from UI
  tests. Do not infer inference readiness from a saved configuration.

## Validation

Run the CLI unit suite and `python -m unittest discover -s workers/media`.
The ignored `workload_picker::tests::terminal_smoke` test can be run under a
PTY without installing models or modifying contributor configuration.
