# Model Lifecycle

This page describes how contributor machines should manage local model cache entries and active model selection.

## Principle

- **Switch, do not delete by default.**
- Keep model files cached locally so contributors do not re-download the same model every time.
- Only remove models when the user explicitly asks, or when disk space is low and pruning is needed.

## Current Prototype Shape

The current config already carries:

- `models` - the models the machine knows about
- `model_dir` - the directory where the machine keeps model files
- `active_model` - the currently selected model, when one is active

On the node agent side, the worker already reads an effective model directory from config.

The official starter model presets are defined in:

- [apps/cli/config/official-models.json](/Users/DBATALL/Documents/mundusx/apps/cli/config/official-models.json)

That file is the reviewable source of truth for the default `start` / `connect` model choices on the CLI.

Those starter presets now point at public Hugging Face GGUF model files compatible with the local Mac runtime, so the default path does **not** need a Hugging Face account.
If a model is missing, the CLI downloads the public GGUF file, verifies the checksum when one is provided, and then caches it locally.

The CLI now has working local cache commands that operate on a manifest directory inside the model cache:

- `opengpu model list`
- `opengpu model use <name>`
- `opengpu model add <name>`
- `opengpu model import <path> --name <name>`
- `opengpu model remove <name>`
- `opengpu model prune --yes`

The terminal output for these commands is intentionally styled like a compact retro operator panel so the active model and cache state are easy to scan quickly.

## Recommended Contributor Flow

### First run

1. `opengpu start` or `opengpu connect` bootstraps the machine.
2. The CLI checks whether the selected open model is already cached.
3. If the model is missing, it downloads the public Hugging Face model file first.
4. When a checksum is present in the catalog, the CLI verifies the downloaded file before activating it.
5. The selected model becomes the active model.

### Switching models

Use a command like:

```bash
opengpu model use Qwen/Qwen2.5-1.5B-Instruct
```

Behavior:

- If the model is already cached, switch immediately.
- If it is missing and the model is one of the official open presets, download it first, then switch.
- Do not delete the old model automatically.

### Downloading a new model

Use a command like:

```bash
opengpu model add HuggingFaceTB/SmolLM2-135M-Instruct
```

Behavior:

- Add the model to the local cache.
- If the model is one of the official open presets, download it from Hugging Face first.
- Before downloading on CUDA machines, compare the catalog's estimated VRAM requirement against the selected contribution cap applied to detected GPU VRAM and refuse models that do not fit.
- Refuse CUDA catalog downloads when required size metadata is missing instead of guessing.
- Do not make it active unless the user also asks to use it.

### Importing a local model

Use a command like:

```bash
opengpu model import ./models/local-q4_k_m.gguf --name local-q4 --backend cuda --vram-mb 4096 --activate
```

Behavior:

- Record the existing file path instead of downloading a catalog model.
- Capture file name, format, quantization tag, size, estimated VRAM, and compatibility status in the local manifest.
- Mark CUDA models as accepted, degraded, or rejected against the supplied VRAM budget.
- Use the selected contribution cap applied to detected CUDA VRAM when `--vram-mb` is omitted.
- Do not activate rejected imports.
- Keep unsupported formats rejected until the local worker can run them.

### Removing a model

Use a command like:

```bash
opengpu model remove Qwen/Qwen2.5-1.5B-Instruct
```

Behavior:

- Remove the model only if it is not active.
- If the model is active, require a switch first or a `--force` flag.

### Pruning unused models

Use a command like:

```bash
opengpu model prune
```

Behavior:

- Remove cached models that are no longer active.
- Keep recently used models unless disk pressure requires more aggressive cleanup.

## Deletion Rules

- **Never delete the active model automatically.**
- **Never delete old models just because a new one was chosen.**
- **Delete only on explicit user request or explicit prune policy.**

## Why This Is The Right Default

- Faster switching
- Less re-downloading
- Safer rollback
- Fewer surprises for contributors

## What To Build Next

- `M` worker adapter
- health checks for the Mac worker backend
