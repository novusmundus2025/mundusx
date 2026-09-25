# Media contribution

## Video workflow

The web service queues authenticated text-to-video requests at `/video`.
Contributors use Wan2.2-TI2V-5B through a pinned ComfyUI workflow. Qwen-Image
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

All presets use 1280 x 704, 24 fps, 20 steps, CFG 5, UniPC/simple, and
ModelSamplingSD3 shift 8. Outputs are silent H.264 MP4 files. The latent frame
constraint gives durations slightly longer than their nominal labels.

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

Production queue-to-download acceptance and public release publication must be
recorded separately; local generation alone does not establish either.

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
