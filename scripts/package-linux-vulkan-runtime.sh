#!/usr/bin/env bash
set -euo pipefail
output="${1:-llama-runtime-x86_64-unknown-linux-gnu-vulkan.tar.gz}"
work="$(mktemp -d)"
git clone --branch b9856 --depth 1 https://github.com/ggml-org/llama.cpp.git "$work/source"
cmake -S "$work/source" -B "$work/build" -DGGML_VULKAN=ON -DGGML_CUDA=OFF -DGGML_NATIVE=OFF -DLLAMA_CURL=OFF -DCMAKE_BUILD_TYPE=Release -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON -DCMAKE_INSTALL_RPATH='$ORIGIN'
cmake --build "$work/build" --config Release -j 2 --target llama-cli llama-server
mkdir "$work/bundle"
cp -L "$work/build/bin/llama-cli" "$work/build/bin/llama-server" "$work/bundle/"
find "$work/build" -type f -name '*.so*' -exec cp -L {} "$work/bundle/" \;
tar -czf "$output" -C "$work/bundle" .
sha256sum "$output" > "$output.sha256"
