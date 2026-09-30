# Media contribution

The installer enables **Images at 32 GiB** and **Videos at 64 GiB** of memory
after the contribution cap, matching the bundled Qwen Image and Wan 14B profiles.
The picker, scripted workload options, setup, and readiness checks all enforce
these model-specific thresholds on macOS, Windows, and Linux.

All platforms apply a 24 GiB minimum **contributed** memory budget to image,
editing, video, and image-to-video selection. The cap is chosen before workloads;
32 GiB at 50% is ineligible, while 32 GiB at 75% meets this general threshold.
Model-specific minima still apply (and may be higher). Scripted selection,
direct media setup/generation, readiness certificates and video queue serving
enforce this rule as well. See `docs/install-strategy.md` for the distribution paths.

## Video workflow

The web service queues authenticated text-to-video requests at `/video`.
Contributors use Wan2.2-T2V-A14B through a pinned ComfyUI workflow. Qwen-Image
remains the separate image model; neither image editing nor image-to-video is
implemented by the video worker.

```sh
opengpu install --workloads llm,image,video --setup-media --yes
opengpu media --video verify
opengpu start
```

An existing installation can retain its LLM configuration and identity. Legacy
configurations default to LLM only. `all` saves all workload preferences, but
unsupported editing operations are not advertised as executable. Managed setup
currently targets Linux ARM64 GX10. Existing ComfyUI can be inspected with
`--comfyui-url http://127.0.0.1:8188`; it must contain the pinned workflow's models
and nodes. External runtimes are never automatically stopped.

The node launches the video listener for the production control plane. A custom
control plane must explicitly set `MUNDUSX_MEDIA_SERVER_URL` to its corresponding
web service. UAT is not silently mapped to production. An admitted, registered
contributor can also run `opengpu media serve --server https://chat.mundusx.ai`.
The current admission contract still requires a healthy LLM runtime; standalone
media-only network admission is not implemented.

## Scheduling and compatibility

Video jobs use a separate PostgreSQL queue with renewable, fenced leases. Legacy
LLM workers do not receive video jobs. Requests are FIFO among supported presets,
with up to four pending requests per account and one active video job per node.
Expired leases retry up to three attempts. Worker requests are signed with the
existing device identity and protected against replay.

Before local generation, the media helper requests a drain. The agent stops
accepting new LLM requests, waits for active requests, and releases only its owned
runtime before acknowledging. Concurrent LLM requests remain supported while no
media job is running. Local API slots also obey the drain. Owned LLM runtime
resumes after generation. External clusters require an explicit ownership
arrangement and are not automatically evicted.

## Presets and accounting

The 14B replacement uses 1280 x 704, 16 fps, 20 steps, CFG 3.5,
Euler/simple and ModelSamplingSD3 shift 8. The high-noise expert runs steps
0–10, then the low-noise expert continues the same latent for steps 10–20
without adding new noise. Both diffusion experts use the official scaled FP8
weights, the UMT5 text encoder is shared, and the VAE is `wan_2.1_vae`.
The 2/5/10-second presets have 33/81/161 frames. The latent frame constraint
gives durations slightly longer than their nominal labels. Outputs are silent
H.264 MP4 files. Managed setup requires a 64 GiB contribution budget.

Workflow reference: Comfy-Org/workflow_templates revision
`5d6089c4250f59b66040651ad100de99d4fcb8ca`,
`templates/video_wan2_2_14B_t2v.json` (non-distilled two-expert workflow).

The GX10 at 100% contribution rendered the 33-frame preset in 893,905 ms
(14 minutes 54 seconds). Its 1,494,263-byte MP4 passed full decode validation;
an extracted frame was inspected. SHA-256:
`504a8bd403cc406be86d723e1b5a196f5893cf15752ad22a08b3b6377e28a11f`.
This yields 15 contributor credits for the 2-second preset. Initial 5/10-second
quotes are 37/73 credits, extrapolated linearly by frame count from this run,
not independently measured benchmarks. Their API quotes and UI explicitly mark
them provisional. Neither generation time nor visual quality is guaranteed.

Models live in `~/.opengpu/media/models/`. The shared UMT5 encoder and Qwen image
files are retained during upgrade. The retired 5B diffusion weights and
`wan2.2_vae.safetensors` can be removed after 14B verification and cutover.
On the test GX10 these two retired files were removed after verification,
freeing 11,409,059,808 bytes. Qwen image files and the shared encoder were retained.
Linux release `cli-linux-v0.2.15` was published from `ecb42cf`; all 18 release
asset digests and all 16 Linux production-channel asset digests matched the
locally built files. The companion web deployment is commit `a7234fb`.

### Historical 5B pricing

These retired 24 FPS presets remain recognizable for historical jobs and credits.
New requests use distinct 14B profile IDs; 5B workers cannot claim 14B jobs.

| Label | Frames | Reference generation | Fixed contributor credits |
| --- | --- | --- | --- |
| 2 seconds | 49 | 208.508 seconds | 4 |
| 5 seconds | 121 | 535.361 seconds | 9 |
| 10 seconds | 241 | 1212.278 seconds | 21 |

These are provisional single-GX10 measurements, not throughput guarantees.
Credits use `ceil(reference_generation_ms / 60000)`, quoted and stored when the
job is submitted. A slower node or a lower contribution setting cannot inflate
its reward. This is contributor accounting, not customer balance debiting.

```sh
opengpu media --video --seconds 5 generate --prompt "A gentle camera move across a landscape"
opengpu media --video --seconds 10 generate --prompt "Clouds move above a quiet mountain lake"
```

The worker uploads with a scoped ticket. The server checks the hash, size,
codec, dimensions, frame count, frame rate and full decode before completion.
Successful settlement writes one durable credit record; completion retries are
idempotent. Pending completion survives a worker restart.

The requesting user receives an unguessable download link. Download requires no
login. Files expire and are deleted after 48 hours; jobs and credit records remain.
No operator credential or device private key is placed in a download or upload URL.

## GX10 acceptance evidence, 2026-09-25

The 5-second preset completed at 65% contribution with 24,504,524,800 bytes peak
container memory and 43,173,117,952 bytes peak system usage. Output: 238,999 bytes,
SHA-256 `f802551adc9deaa61540e45547cb26a51661a5e75623b359331762ca7ecaaf70`.

The 10-second preset completed at 65% contribution with 28,447,268,864 bytes peak
container memory and 46,570,246,144 bytes peak system usage. Output: 286,453 bytes,
SHA-256 `de78bb3f8f0f0ac1a2eb27a453eeaded69b63b54e0728855b2d5ad0c59266858`.
Midpoint frames were inspected; generated object geometry can vary over time.

At the user's requested 100% contribution, the 2-second verification completed in
182,667 ms. Output: 187,622 bytes, SHA-256
`9c154ebabba64014568de324ff4b73e9dd6a5e139e5763c3a721b231090d3e09`.

Production job `242fc5ab-c01d-4968-b692-134718559c3f` completed on the GX10 at
100% contribution. The owned LLM runtime stopped before generation and restarted
afterward. Generation took 224,635 ms. The anonymous MP4 download returned HTTP 200,
120,522 bytes, and SHA-256
`3b3d3fc61305626fd9e1cb51d289e9acf6a58ae214fdcc1a03f8e7b105586b6a`.
PostgreSQL contained exactly one 4-credit video reward. The server expiry was
2026-09-27 07:56:47 UTC; job and reward retention were tested independently.

Linux release 0.2.14 is published in `mundusx/releases`, with ARM64 and x86_64
binaries and SHA-256 checksums. All 18 uploaded release asset digests matched the
local files. The GX10's normal `opengpu update` downloaded and verified that public
release. Native ARM64 generation and emulated x86_64 command startup were checked;
x86_64 GPU inference was not hardware-tested. The hosted build could not start
because of the private repository's Actions budget, so these binaries were built
locally from commit `33ee5b2`.

The public build passed 210 CLI, 139 node, 19 media-helper and 425 web tests.

## Checks

```sh
cargo test -p opengpu --bin opengpu -- --test-threads=1
cargo test -p opengpu-node-agent --bin opengpu-node-agent -- --test-threads=1
python -m unittest discover -s workers/media
```

Node tests run serially because existing tests modify shared environment state.
Web tests cover authenticated requests, ownership, replay protection, FIFO claims,
lease expiry, MP4 validation, downloads, reward idempotency, and file expiry with
retained job/credit records. Hardware testing of every OS, arbitrary external
ComfyUI installation, editing, and longer-than-10-second workflows remains outside
this release.
