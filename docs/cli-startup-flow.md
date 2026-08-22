# MundusX CLI Startup Flow

This document describes what the CLI does when a user starts using it for the first time and what each command is responsible for.

## First Run

The CLI owns the user-facing lifecycle. It manages local state and, when the user runs `opengpu start`, launches the installed node agent that connects the machine to the platform.

On first run, the CLI:

1. Creates a local config file if needed.
2. Generates a device ID.
3. Reuses an existing local secure identity if present, or creates one on first run.
4. Sets default values for:
   - connection state
   - pause state
   - backend preference
   - control-plane URL
   - contribution cap placeholder
5. Stores the config in the preferred config directory, or falls back to a local `.opengpu/config.json` file if needed.
6. If `OPENGPU_HOME` is set, that path wins over any repo-local fallback.

## Install

The platform bootstrapper installs the native `opengpu` binary only:

- macOS/Linux use `install.sh`.
- Windows uses `install.ps1`.
- The bootstrapper selects the release asset for the operating system and CPU architecture.
- The full contributor setup always happens inside `opengpu install`.

When the user runs:

```bash
opengpu install
```

the CLI runs the guided machine setup wizard:

1. Creates local config and secure device identity if needed.
2. Detects the machine profile, including OS, CPU architecture, Apple Silicon or CUDA backend, NVIDIA GPU name, and CUDA VRAM when available.
3. Probes the machine for a local LLM cluster that is already running, so the
   cluster step later in the wizard has results ready.
4. Asks which control plane to use:
   - public MundusX, which saves the hosted MundusX API endpoint
   - private / custom, which asks for a full `http://` or `https://` URL
   - blank control-plane input defaults to public MundusX
5. Asks how much of this machine's compute budget MundusX may use, with quick
   picks, custom whole-percent entry from `1%` through `80%`, and — when a
   running cluster was detected — a final `Clusters detected` row:

   ```
   Contribution level
   -------------------
      20% - light
   >> 30% - balanced
      50% - strong
      65% - high
      80% - maximum
      custom - type exact percent (1-80)
      Clusters detected (2) - show all and pick one to contribute
   ```

   Choosing that row lists every running cluster, biggest model first, plus a
   `None` row:

   ```
   A local LLM cluster is already running on this machine.
   Contribute one to MundusX instead of downloading a model?
   ----------------------------------------------------------
   >> llama.cpp at http://127.0.0.1:8000 - UD-IQ2_M (754B params, 222.2 GB); 1 available
      Ollama at http://127.0.0.1:11434 - hermes3:70b (70.6B params, 37.2 GB); 2 available
      None - do not contribute a cluster; choose a MundusX model instead
   ```

   Picking a cluster contributes it and finishes setup; the default cap is saved
   because local policy requires one, even though the cap does not gate a
   contributed cluster. `None` or `Esc` returns to the contribution level menu.
   See [Contributing a Running Cluster](#contributing-a-running-cluster).
7. Asks which model this node should run, unless a running cluster was
   contributed in the previous step:
   - lighter safe catalog model
   - recommended safe catalog model
   - local GGUF / LM Studio model file
8. Filters model choices by the selected contribution cap and detected machine profile, then checks model fit before download or activation.
9. Saves the selected control-plane URL, contribution cap, and active model or contributed cluster.
10. Prints the next step: `opengpu start`.

Scripted installs can pass flags instead of using the prompts:

```bash
opengpu install --public --cap-percent 30
opengpu install --private --control-plane-url http://127.0.0.1:8787 --cap-percent 30
opengpu install --public --cap-percent 30 --contribute-cluster
opengpu install --public --cap-percent 30 --no-contribute-cluster
```

## Contributing a Running Cluster

Many contributors already run a local LLM before they ever install MundusX.
Standing up a second runtime beside it wastes VRAM, competes for the same GPU,
and downloads weights the machine already has. So `opengpu install` and
`opengpu start` look for that cluster first and offer to contribute it.

Detection probes these localhost endpoints, in order, with a two second timeout
each:

| Runtime | Port | Listing endpoint |
| --- | --- | --- |
| Ollama | `11434` | `/api/tags` |
| LM Studio | `1234` | `/v1/models` |
| vLLM | `8000` | `/v1/models` |
| OpenAI-compatible server | `8080` | `/v1/models` |

Port `8789` is the MundusX-managed `llama-server` and is never treated as a
foreign cluster. `OPENGPU_CLUSTER_PROBE_URLS` replaces the list with a
comma-separated set of base URLs, and `OPENGPU_SKIP_CLUSTER_DETECT=1` disables
detection.

The decision then follows one policy shared by `install` and `start`:

- No endpoint answered with a model: nothing is asked and setup continues.
- An endpoint answered but advertises no loaded model: it is reported as
  detected-but-idle and is not listed, because it cannot serve work yet.
- `--contribute-cluster` or `--no-contribute-cluster` was passed: that answer is
  used and no picker is shown.
- A cluster is already contributed, or the contributor already declined: setup
  does not ask again. `opengpu cluster use <url>`, `opengpu cluster forget`, or
  `--contribute-cluster` reopens the decision.
- The run is not interactive: setup never contributes silently. It prints the
  detection and the `--contribute-cluster` hint, then continues normally.
- Otherwise the picker is shown. Choosing a cluster contributes it; choosing
  `None` is recorded as a decline so setup stops asking; `Esc` skips the question
  for this run only, leaving it to be asked again next time.

### Which cluster, and which model

Everything is ranked by size, so the biggest thing on the machine is what gets
offered:

- Within one endpoint, the node advertises the **largest model it serves** - a
  runtime holding both a 70B and an 8B contributes the 70B.
- Across endpoints, the **cluster with the largest model wins**, regardless of
  probe order. An Ollama on `11434` serving an 8B is ranked below a llama.cpp on
  `8000` serving a 753B, even though `11434` is probed first.
- Size comes from whatever the runtime reports: llama.cpp's `meta.n_params` and
  `meta.size`, or Ollama's `details.parameter_size` and `size`. Parameter count
  decides first and on-disk bytes break ties; a model reporting neither sorts
  last rather than being dropped.
- `opengpu cluster use <url> --model <name>` overrides the automatic pick.

### Identifying the runtime

The runtime is identified from its own model listing, never from the port it
occupies: `owned_by` (`llamacpp`, `vllm`), llama.cpp's `meta.n_ctx_train` /
`meta.n_vocab` model metadata, or Ollama's `details` / `digest` fields. The port
is only a hint for which listing path to try first. A llama.cpp server on `8000`
is reported as llama.cpp, not vLLM.

### What the node advertises

A contributed cluster changes what the node reports to the control plane:

| Field | Value |
| --- | --- |
| `runtime_mode` | `contributed-cluster` |
| `capacity_class` | derived from the advertised model: `>=70B` synthesis, `>=30B` heavy, `>=13B` performance, `>=7B` standard, else micro; on-disk size is used when the parameter count is unknown, and the host-memory ladder when neither is reported |
| `usable_vram_mb` | absent - the cluster owns its own memory, so the contribution cap produces no VRAM budget |
| `parallel_slots` | the lower of vLLM's configured `max_num_seqs` and its full-context KV-cache concurrency; if `/server_info` is unavailable, KV capacity is safety-capped at 16; non-reporting runtimes fall back to 1 |
| `roles` | `reducer` at heavy-or-above and `synthesizer` at synthesis-or-above, from that capacity class |

Roles matter because the control plane gates `reducer` and `synthesizer`
strictly on role membership while other roles fall back to an empty role list.
Deriving them from the capacity class is what keeps a large contributed cluster
eligible for the heavy graph nodes; the VRAM and slot thresholds used for
MundusX-managed runtimes can never fire for a contributed cluster.

Health for these nodes is the endpoint answering `/v1/models`, `/api/tags`, or
`/health`, plus an advertised model - not a local `llama-cli` or model file.

Contributing a cluster records its kind, endpoint, model list, and adoption time
in `config.json`, then skips both the MundusX model selector and runtime
provisioning. The cluster's model becomes the node's effective active model, so
`status`, `doctor`, the startup summary, and the default model for `run` and
`jobs submit` all resolve to it while the local model cache stays empty.

Declining is remembered so setup stops asking on every run.

## Login

When the user runs:

```bash
opengpu login
```

the CLI:

1. Saves an operator bearer token locally. On Windows, the token is protected with DPAPI outside `config.json`.
2. Keeps the machine ready for future control-plane calls.
3. Supports either an explicit `--token` value or an interactive prompt.

## Start

When the user runs:

```bash
opengpu start
```

the CLI:

1. Creates local config if needed.
2. Connects the machine locally when the secure device identity is available.
3. Clears the paused state only when the node can sign requests.
4. Detects the machine backend when possible:
   - Apple Silicon `aarch64` on macOS becomes `M`
   - NVIDIA CUDA machines become `CUDA` when CUDA environment hints or `nvidia-smi` are available
5. Optionally overrides that with `--m`.
6. If no contribution cap is saved yet, asks the user to choose the budget on interactive terminals.
   - `20%` light
   - `30%` balanced
   - `50%` strong
   - `65%` high
   - `80%` maximum
   - custom whole-percent entry from `1%` through `80%`
   - use the arrow keys and press Enter to confirm
   - press `Ctrl-C` to cancel the cap selector cleanly
   - scripted or non-interactive runs still print the `opengpu cap` hint instead of choosing silently
7. Probes for a running local LLM cluster and, when one is found and no decision
   is recorded yet, asks whether to contribute it. `--contribute-cluster`,
   `--no-contribute-cluster`, and `--cluster-url` work the same way as on
   `install`. See [Contributing a Running Cluster](#contributing-a-running-cluster).
8. If no active model is saved yet and no cluster is contributed, asks for a model choice, caches or imports it, and marks it active.
   - the starter model presets come from `apps/cli/config/official-models.json`
   - the starter presets point at public Hugging Face GGUF files compatible with the local Mac runtime, so no account is required for the default path
   - if the selected model is missing, the CLI downloads the public GGUF file and verifies the checksum when one is present in the catalog
9. Prints a startup summary with:
   - device ID
   - public key
   - public key fingerprint
   - identity readiness
   - platform
   - CPU core count
   - backend preference
   - detected backend when `backend preference` is `auto`
   - contributed cluster when one was adopted
   - active model
   - model cache directory
   - contribution percent
   - connection state
   - pause state
   - config path
   - power source
   - battery percent
   - policy allowance
   - policy reason when the Mac should stay quiet
10. Prints the contributor onboarding checklist until it is marked complete:
   - device identity
   - hostname
   - backend
   - active model
   - contribution cap
   - policy state
   - credits link
   - dashboard link
11. Saves the updated config.
12. Prints how the contribution cap should be interpreted:
   - `M` means a memory-and-compute budget on Apple Silicon
   - `CUDA` means a routing budget for NVIDIA nodes, with low-VRAM cards kept to modest workloads
13. Prints whether policy currently allows the Mac to accept work, including the power source, battery state, and identity readiness.
14. Keeps the reused secure device identity attached to the local config when available.
15. Starts the installed `opengpu-node-agent` companion binary in a foreground contribution session when the secure device identity is ready.
16. Keeps the terminal attached so status and worker logs remain visible.
17. Treats `Esc` or `Ctrl-C` as a graceful disconnect: local state is saved as disconnected and paused, the node agent is stopped, and any warm `llama-server` runtime is cooled so GPU memory is released.

For daemon mode, run:

```bash
opengpu start --background
```

For explicit foreground diagnostics, run:

```bash
opengpu start --debug
```

Debug mode keeps the node agent attached to the terminal and prints the underlying agent logs. Direct `opengpu-node-agent run` remains an internal/developer entry point, not the normal contributor command.

## Operator Auth

If the control plane starts with `OPENGPU_OPERATOR_TOKEN`, the human-facing API routes and browser health page require a matching bearer token. The CLI stores that token locally with `opengpu login`, and clears it with `opengpu logout`. On Windows, the token is stored as a DPAPI-protected blob in the OpenGPU config directory, while `config.json` stays non-secret and safe to inspect.

## Connect

When the user runs:

```bash
opengpu connect
```

the CLI:

1. Marks the local config as connected.
2. Clears the paused state.
3. Reuses the active model cache if one already exists.
4. Prepares the machine to participate in routing once the control plane is online.
5. Reuses the active local model entry if it is already configured.
   - if the selected open model is missing, it is downloaded before being marked active

## Disconnect / Exit

When the user runs:

```bash
opengpu exit
```

`opengpu disconnect` remains available as the explicit advanced form.

the CLI:

1. Marks the local config as disconnected.
2. Pauses contribution.
3. Stops the recorded node agent when one is running.
4. Cools any persistent `llama-server` runtime owned by that agent so the GPU can return to idle power.
5. Leaves the identity and config in place for the next `start`.

## Status

When the user runs:

```bash
opengpu status
```

the CLI:

1. Reads the current local config.
2. Resolves the machine backend if `backend preference` is `auto`.
3. Treats the current machine as the active provider when connected and not paused.
4. Prints local provider state, active model, model cache directory, and the active backend decision.
5. Keeps sample node inventory out of the main status view for now.

## Onboarding

When onboarding has not been completed yet, `opengpu start` and `opengpu connect` print a dedicated checklist panel and a hint to run:

```bash
opengpu onboarding --complete
```

The onboarding command only marks the review step as complete; it does not change the device identity or control-plane state.

When the contribution cap has not been saved yet, `opengpu start` asks for it on an interactive terminal. `opengpu cap` remains the explicit command for changing or resetting it later.

Those deeper network behaviors will come later when the control plane and node agent are online.
