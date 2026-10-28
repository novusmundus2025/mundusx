# Local image-edit quality and dimensions

Run on the contributor with the source PNG present locally:

```bash
opengpu media --image-edit --quality --preserve-size generate --input-image /home/a/artifacts/jes1.png --prompt "Replace the background with a garden; preserve the person and clothing"
```

- `--preserve-size` keeps exact original pixel dimensions. It pads to the next multiple of 64 for inference and removes only that added padding before saving. Inputs already aligned to 64 are passed through without resizing. It never silently downscales a large source.
- `--quality` uses standard sampling, CFG 4, and 30 steps, without the Lightning LoRA. It reuses the installed FP8 base model; it does not convert the model to BF16 or guarantee unchanged pixels in an edited image.
- `--steps 40` selects 20–50 steps in quality mode. More steps increase runtime and do not guarantee a better image.
- `--width 1024 --height 1536` chooses an explicit canvas instead of original size. Both values must be multiples of 64 between 64 and 2048. The input is fitted with padding, preserving its aspect ratio.
- The processing canvas is limited to 2,097,152 pixels and 2048 per side. Larger original images fail with an explanation; no lossy fallback is applied.
- Output is PNG. Source PNG remains unchanged. Generative edits do not guarantee exact identity, color, alpha, or metadata preservation.

Preview settings without a GPU generation:

```bash
opengpu media --image-edit --quality --preserve-size plan --input-image /home/a/artifacts/jes1.png
```

Local custom quality/size verification records are separate from the fixed, network-qualified 832×480 four-step profile. Network jobs retain their existing preset. This does not enable these flags in the network image-edit API.

## Second reference image

The local CLI accepts two PNG files: one primary scene and one optional reference.

```bash
opengpu media --image-edit --quality --preserve-size --reference-image /path/to/palette.png generate --input-image /path/to/scene.png --prompt "Use the second image's color palette; retain the objects and composition in the first image"
```

`--input-image` is picture 1. It supplies the initial latent and determines original-size output. `--reference-image` is picture 2; it is passed separately to Qwen Image Edit Plus conditioning and is not stretched to the scene dimensions. The model internally scales conditioning images. Reference conditioning does not guarantee an exact identity or pixel match.

Both PNGs are validated before renderer uploads. An unsupported renderer fails before either upload. Only one reference is accepted, and the flag is limited to image-edit plan, verify and generate. It does not enable multiple references for image-to-video or the public network API. Local reference runs use separate verification records from the network-qualified preset.

Verified: CLI parsing, distinct uploads and conditioning nodes, primary-owned output dimensions, invalid image rejection, renderer compatibility checks, and the GX10 two-image plan. A full two-image GPU render remains unqualified.
