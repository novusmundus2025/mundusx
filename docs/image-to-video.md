# Local image-to-video on GX10

The CLI supports `--image-to-video` with a separate pinned Wan2.2 I2V A14B
profile. Local generation supports both standard and fast I2V. Queue serving
supports the fast profile with the companion media-only control-plane update.
Use the updated CLI built from this checkout, not an earlier installed binary.

Keep contribution paused and stop externally managed vLLM recipes before setup
and generation. TP2 recipes can reserve memory on both GX10s.

```bash
opengpu media --image-to-video plan
opengpu media --image-to-video setup --yes
opengpu media --image-to-video verify --input-image /absolute/path/input.png
opengpu media --image-to-video --seconds 2 generate --input-image /absolute/path/input.png --prompt "The subject slowly turns, steady camera"
opengpu media --image-to-video status
```

The fast preset fits the whole input with centered black padding when aspect
ratios differ; it never center-crops the input.

Input is a non-interlaced 8-bit RGB/RGBA PNG, at most 32 MiB, 8192 pixels per
axis and 16 megapixels. ComfyUI center-resizes it to the profile dimensions.
The two I2V diffusion weights require about 26.6 GiB additional download;
the existing UMT5 text encoder and VAE are reused after checksum validation.
Runtime and verification records are separate from text-to-video and images.
The existing media lock serializes all image/video operations. Output MP4s
receive the same codec, dimension, frame-count and full-decode validation.

GX10 acceptance on 2026-10-04: all 44 media tests passed on the host. The
updated CLI was built and installed at `/home/a/.local/bin/opengpu`, with the
previous binary backed up at
`/home/a/opengpu-builds/i2v-20261004/opengpu-before-i2v`.
The existing verified teapot PNG produced a visually inspected rotating teapot:
1280x704 H.264 MP4, 33 frames at 16 fps, 159,884 bytes, in 907,718 ms.
SHA-256: `ae2d1d7a63de1524dd76726359b3ef4e2e6889a92a194ff7e2377c26343c5305`.
The artifact is `/home/a/.opengpu/media/artifacts/ae9f215d7ab84e7bbb5e90ec7dfa705a.mp4`.
`media --image-to-video status` reports `ready: true`. The owned container was
removed and available host memory returned to 116 GiB. First, middle and last
frames were inspected. This validates one short preset, not all durations or
simultaneous TP2/vLLM operation. The earlier acceptance run was local; queue serving requires the companion
control-plane update and media-only admission.

## Fast I2V on GB10

Use `--image-to-video --fast` for the separate Wan2.2 Lightning preset:
832x480, four steps split 2+2, CFG 1.0, shift 5 and 16 fps. Ten seconds uses
161 frames. Omitting `--fast` retains the standard 1280x704, 20-step preset.

```bash
opengpu media --image-to-video --fast setup --yes
opengpu media --image-to-video --fast --seconds 10 verify --input-image /absolute/path/input.png
opengpu media --image-to-video --fast --seconds 10 generate --input-image /home/a/.opengpu/media/artifacts/26.png --prompt "The subject slowly turns, steady camera"
opengpu media --image-to-video --fast --seconds 10 status
```

The high/low expert LoRAs come from `lightx2v/Wan2.2-Lightning`, revision
`18bccf8884ec0a078eed79785eb4ef13ea16ce1e`, directory
`Wan2.2-I2V-A14B-4steps-lora-rank64-Seko-V1`. Each download is SHA-256 checked
and applied to its matching expert at strength 1.0. The two additional files
total about 2.29 GiB. Runtime and verification records use the separate
`-i2v-fast` suffix; verification is specific to duration.

Managed ComfyUI now accounts for reclaimable system cache on GB10 when
reporting available CUDA memory. This bootstrap applies only to OpenGPU-owned
GB10 processes; the Docker memory limit and ComfyUI reserve remain in place.
It does not modify external ComfyUI installations or discrete-GPU accounting.

Fast acceptance on 2026-10-04: 48 media tests passed locally and on GX10,
and the CLI flag-validation test passed. A two-second teapot test completed
in 146,360 ms, compared with 907,718 ms for the earlier standard preset.
This comparison combines resolution, sampler and memory-accounting changes;
it does not isolate the effect of LoRA. The first, middle and last frames
retain the subject and scene, with modest motion. Lower resolution and
fewer steps can reduce detail and motion quality; timing is scene dependent.
The installed binary backup is
`/home/a/opengpu-builds/i2v-20261004/opengpu-before-fast`.

On 2026-10-05 the fast preset changed to 1280x704 with aspect-preserving padding.
The earlier 146-second benchmark was at 832x480 and does not predict this preset.
No new generation benchmark was run at the user's request. Run setup again
to refresh the profile record; old verification certificates do not apply.

The fast preset was returned to 832x480 at the user's request, retaining
four-step Lightning and aspect-preserving padding (profile v3).
