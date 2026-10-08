# Muse Glimmer contributor selection

The Linux vLLM model picker offers both variants:

| Variant | Repository | Estimated serving budget |
| --- | --- | --- |
| FP8 | `RedHatAI/Muse-Glimmer-30B-FP8-block` | 48 GiB (provisional) |
| BF16 | `meta-models/Muse-Glimmer-30B` | 72 GiB |

Existing recommended models and active selections are unchanged. Budgets apply after the contribution cap and are admission estimates, not measured peaks or guarantees for every workload. Neither variant is offered for MLX, llama.cpp CUDA, or Vulkan.

Muse Glimmer is a chat/agent model producing text. It is not an image or video generator. The model's image-understanding capability does not by itself enable image input through every MundusX API or mark a contributor as vision-ready.

## Runtime

The downloader and node worker use the same pinned multi-platform `vllm/vllm-openai:v0.28.0` image for this model, instead of the older NVIDIA image in the installer's runtime.conf. An explicit `OPENGPU_VLLM_IMAGE` environment override still takes precedence and must support Muse Glimmer. Other models retain the existing runtime selection.

Both variants enable the `muse_glimmer` tool-call and reasoning parsers and load the model's generation configuration. Initial context is limited to 8192 tokens; the model's published 128K context is not a promise that this contributor configuration serves 128K. No speculative drafter is downloaded. Existing memory utilization and concurrency settings still apply. FP8 containers disable DeepGEMM to avoid the SM120 block-layout incompatibility documented in the vLLM recipe; BF16 is unchanged.

After this change is built into an installed CLI and node worker, use `opengpu model use` and select **Muse Glimmer 30B — FP8** or **Muse Glimmer 30B — BF16**. Alternatively, choose one of these repository IDs:

```bash
opengpu model use RedHatAI/Muse-Glimmer-30B-FP8-block
```

```bash
opengpu model use meta-models/Muse-Glimmer-30B
```

Selecting FP8 downloads approximately 33 GB of weights; BF16 approximately 60 GB, plus the shared runtime image. This source change does not switch or download models on an existing contributor automatically.

## Validation status

Catalog memory/backend filtering and runtime selection are covered by Rust tests. The image's registry manifest includes Linux ARM64 and AMD64. A full generation, tool-calling, image-input, and LLM/media handoff test on GX10 remains required before claiming hardware validation. This change is not part of the published 0.2.15 binaries.

Sources:
- https://huggingface.co/meta-models/Muse-Glimmer-30B
- https://huggingface.co/RedHatAI/Muse-Glimmer-30B-FP8-block
- https://recipes.vllm.ai/meta-models/Muse-Glimmer-30B
