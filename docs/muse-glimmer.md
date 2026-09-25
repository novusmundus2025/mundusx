# Muse Glimmer contributor selection

The Linux vLLM model picker includes `meta-models/Muse-Glimmer-30B` as an optional BF16 model. Existing recommended models and active selections are unchanged. The entry requires an estimated 72 GiB of cap-adjusted memory; this is a conservative admission estimate, not a measured peak or a guarantee for every workload. It is not offered for MLX, llama.cpp CUDA, or Vulkan.

Muse Glimmer is a chat/agent model producing text. It is not an image or video generator. The model's image-understanding capability does not by itself enable image input through every MundusX API or mark a contributor as vision-ready.

## Runtime

The downloader and node worker use the same pinned multi-platform `vllm/vllm-openai:v0.28.0` image for this model, instead of the older NVIDIA image in the installer's runtime.conf. An explicit `OPENGPU_VLLM_IMAGE` environment override still takes precedence and must support Muse Glimmer. Other models retain the existing runtime selection.

The worker enables both `muse_glimmer` tool-call and reasoning parsers and loads the model's generation configuration. Initial context is limited to 8192 tokens; the model's published 128K context is not a promise that this contributor configuration serves 128K. No speculative drafter is downloaded. Existing memory utilization and concurrency settings still apply.

After this change is built into an installed CLI and node worker, use `opengpu model use` and select **Muse Glimmer 30B — BF16**, or select the repository ID directly:

```bash
opengpu model use meta-models/Muse-Glimmer-30B
```

Selecting the model can download approximately 60 GB of weights plus its runtime image. This source change does not switch or download models on an existing contributor automatically.

## Validation status

Catalog memory/backend filtering and runtime selection are covered by Rust tests. The image's registry manifest includes Linux ARM64 and AMD64. A full generation, tool-calling, image-input, and LLM/media handoff test on GX10 remains required before claiming hardware validation. This change is not part of the published 0.2.15 binaries.

Sources:
- https://huggingface.co/meta-models/Muse-Glimmer-30B
- https://recipes.vllm.ai/meta-models/Muse-Glimmer-30B
