# Optional image and video contribution

First complete the [platform installation](getting-started.md) and
[contributor setup](contributor-setup.md). This guide covers the bundled Qwen
image and Wan video profiles, not arbitrary ComfyUI workflows or custom nodes.

## 1. Check memory and select workloads

Images require at least **32 GiB after the contribution cap**; videos require
**64 GiB after the cap**, on every OS. A 32 GiB machine is therefore not image
eligible at the maximum 80% cap. Review the memory table in the setup guide.

Run `opengpu install` to select the eligible workloads. Lowering the cap later
can invalidate readiness and require verification again.

## 2. Choose the media runtime

| Platform | Managed setup path |
| --- | --- |
| Linux ARM64 GB10/GX10 | Pinned ComfyUI Docker runtime |
| Windows x86_64 with NVIDIA | Native ComfyUI using CUDA; Python 3.12 or 3.13 required |
| Apple Silicon Mac | Native ComfyUI using MPS/Metal; Python 3.12 or 3.13 required |
| Other supported machines | Use an existing compatible ComfyUI endpoint |

Windows/macOS native setup is implemented, but release/build tests do not prove
that every bundled model works on every GPU. Only successful local generation
verification establishes readiness. MLX serves LLMs; Mac ComfyUI uses MPS.

Setup may detect existing ComfyUI and offer it. To supply an endpoint explicitly:

```text
opengpu install --comfyui-url http://127.0.0.1:8188
```

Use its actual port. MundusX does not take ownership of an external ComfyUI
process or stop it. For external ComfyUI verification, pause contribution first
and let LLM jobs finish. Before resuming LLM work, ensure the external server has
released model memory. Managed media drains LLM work around generation.

## 3. Review downloads, install, and verify

The wizard can offer these steps interactively. If you deferred them, run:

```text
opengpu media plan
opengpu media setup --yes
opengpu media verify
opengpu media status
```

For video:

```text
opengpu media --video plan
opengpu media --video setup --yes
opengpu media --video verify
opengpu media --video status
```

`plan` displays the profile, download sizes, and budget. Read it before accepting
downloads with `--yes`. Video also requires `ffmpeg` and `ffprobe` on PATH.
Required free disk space includes missing weights and runtime headroom. Cached
weights are verified and reused when valid.

Interactive downloads show bytes/progress bars; redirected output can show JSON
events. Verification runs an actual generation and reports elapsed time. A saved
configuration or reachable ComfyUI endpoint is not a passing generation test.

## 4. Optional local generation

```text
opengpu media generate --prompt "A red ceramic teapot on a wooden table"
opengpu media --video --seconds 2 generate --prompt "A red ceramic teapot slowly rotates"
```

Release 0.2.20 supports every integer from **1 through 10 seconds** with the
832×480, 16 FPS profile. Local generation reports its output
artifact path. It does not automatically create a public user download URL.

## 5. Start network contribution

```text
opengpu start --background
opengpu status
opengpu capabilities --probe
```

The node agent starts the media queue worker for selected image/video workloads.
The worker claims only verified, memory-eligible profiles. Network media also
depends on control-plane admission; the current admission path requires an
admitted LLM contributor. Media-only local generation is not proof of admission.
Custom deployments must configure their matching media service; normal public
nodes use `https://chat.mundusx.ai`.

For queued image jobs, the worker uploads the generated image to the hosted
media service and completes the job after successful upload. The requesting
user receives the hosted result link through the client service. After a verified
image upload, MundusX permanently removes its local upload copy and, for managed
ComfyUI, the managed output copy. It cannot remove an external ComfyUI server's
original through this integration. Video cleanup behavior is different; do not
assume image deletion applies to video files. Queued service availability still
depends on the corresponding control-plane deployment.

Managed ComfyUI runs on demand. Do not expect it to remain warm and listening on
port 8188 while idle. To stop an interrupted MundusX-owned media runtime:

```text
opengpu media stop
```

This does not stop an externally managed ComfyUI server.
