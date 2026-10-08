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

Release 0.2.39 selects four-step Lightning by default for both `--video` and
`--image-to-video`, including setup, verification and generation. `--fast` remains
accepted explicitly; use `--quality` to select the regular video profile.
Every integer from **1 through 10 seconds** is supported at 832×480 and 16 FPS;
zero, fractional seconds and durations above 10 are rejected.
Local generation reports its output
artifact path. It does not automatically create a public user download URL.

## 5. Start network contribution

On Linux, a media-only contributor using OpenGPU-owned Docker keeps its runtime
after the first successful request. Matching requests reuse the container and
ComfyUI model cache. No dummy generation is run to preload a model. Changing the
profile/model family or memory cap stops the old container before loading the
next one, so image and video models do not accumulate in memory together.

The contributor advertises one shared slot: Ready with zero active jobs while
idle, and Busy from claim through upload and completion. Health probes run in
the background while heartbeats continue from the latest snapshot. A resident
media runtime blocks LLM startup until its owned container has stopped. Changing
the selected workloads drains the current media job before the worker releases
its runtime. Stop the contributor before starting a separate vLLM recipe;
`opengpu media stop` resets the runtime but does not stop queue polling.

The serving worker renews a 120-second local lease every five seconds. Its managed
container checks the lease every five seconds and exits if renewal stops,
and the agent recovers stale owned containers before allowing an LLM handoff.
External ComfyUI endpoints, native runtimes, `serve --once`, and standalone local
generation retain their existing per-request lifecycle. A current node agent is
required for supervised retention; older agents keep per-request execution.

To opt out before starting the contributor, set `OPENGPU_MEDIA_KEEP_WARM=0`.
Generation logs include `runtime_reused` and `runtime_startup_ms`; existing
`duration_ms` continues to measure generation. Actual GPU speedup must be
measured with a cold request and a second, different prompt on the same profile.

```text
opengpu start --background
opengpu status
opengpu capabilities --probe
```

The node agent starts the media queue worker for selected image/video workloads.
The worker claims only verified, memory-eligible profiles. Network media also
depends on control-plane admission. With the companion media-only admission
deployment, a healthy media-only contributor reports exact verified profiles,
memory budget and one execution slot; it does not need an LLM runtime.
Media-only local generation is not proof of admission. Update both the CLI and
node agent, and follow [media-only contribution](media-only-contribution.md).
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

## Private plane behind a Coder gateway

Media worker requests retain their node signatures and also use the saved,
endpoint-scoped gateway credential. Artifact uploads preserve their scoped
bearer ticket and add `Coder-Session-Token` separately. Redirects are disabled;
credentials are not sent to an unrelated server or included in command-line
arguments, container environments, upload tickets, or diagnostics.

For an existing media-only contributor, finish any active job before switching:

```bash
opengpu disconnect
opengpu config control-plane-url "https://YOUR-PRIVATE-PLANE"
# Only when this private gateway requires a session token:
opengpu login --token-header coder-session-token
export MUNDUSX_MEDIA_SERVER_URL="https://YOUR-PRIVATE-PLANE"
opengpu start --no-contribute-cluster --max-jobs 1
```

The agent starts the media worker automatically when media workloads are saved.
Do not start a second `media serve` process alongside that managed worker.
The private deployment must expose `/api/media/worker/*` and artifact upload
routes and admit the contributor identity. If a worker is being run manually,
`opengpu media serve --server "https://YOUR-PRIVATE-PLANE"` uses the same login.

The saved login is scoped to its configured control-plane origin/path. It is
not forwarded to a different media origin. This upload integration supports
Coder session gateway authentication; a gateway bearer credential conflicts
with the artifact's bearer ticket and is rejected explicitly. Public media
uploads without a matching gateway credential keep their existing behavior.

Gateway authentication is optional: omit `opengpu login` for an open gateway.
Without a matching saved credential, no gateway header is added. Signed node
requests and scoped artifact upload tickets remain required in both modes.
