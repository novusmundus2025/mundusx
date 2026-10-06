# AMD Strix Halo: Windows and Linux

Use Qwen3-Coder-30B-A3B-Instruct Q4_K_M GGUF with llama.cpp Vulkan on Ryzen AI Max systems with 64 GB or more shared RAM. Hardware performance validation is pending. This guide enables text inference; ROCm media generation and the NPU require separate validation.

## Connect a local server

Install a Vulkan-enabled llama.cpp runtime, or use LM Studio with its Vulkan runtime. MundusX's Windows runtime is pinned to llama.cpp b9856. On Linux, install graphics drivers, Vulkan development packages, CMake, Git and C++ build tools, then build:

```sh
git clone --branch b9856 --depth 1 https://github.com/ggml-org/llama.cpp.git
cmake -S llama.cpp -B llama.cpp/build -DGGML_VULKAN=ON -DGGML_CUDA=OFF
cmake --build llama.cpp/build --config Release -j
llama.cpp/build/bin/llama-cli --list-devices
```

Download [Qwen3-Coder GGUF](https://huggingface.co/unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF), file Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf. Its SHA-256 is `fadc3e5f8d42bf7e894a785b05082e47daee4df26680389817e2093056f088ad`.

Start the server with your actual model path (Windows uses llama-server.exe):

```sh
llama-server -m /path/to/model.gguf --device Vulkan0 -ngl 99 -c 4096 --host 127.0.0.1 --port 8080
opengpu install --connection direct --cluster-url http://127.0.0.1:8080
```

Select the model the server reports. Confirm Radeon GPU offload in its logs and complete a local generation before contributing. This direct connection works with existing CLI releases. Increase context gradually after measuring memory use; 32K is a test target, not a verified default.

## Managed integration and catalog

Windows packages a Vulkan runtime. Release 0.2.33 adds Linux automatic Vulkan detection and a pinned runtime bundle. On Linux x86_64 run the release installer with `--with-vulkan`; install the distribution's Vulkan loader and Radeon graphics driver first. Set OPENGPU_LLAMA_CLI to an absolute path only when using a separate runtime build. Use llama-server from the same build.

Approve a separate GGUF catalog variant for Windows/Linux and Vulkan, with chat and coding tags and a provisional 26000 MB serving budget. Leave image input, tools and embeddings disabled pending integration validation. The weights are approximately 18.6 GB; leave memory for the OS, runtime and KV cache. Count shared RAM once, rather than adding GPU memory to system memory.

ROCm/HIP remains a Linux alternative to benchmark against Vulkan on the same model, hardware and context.
