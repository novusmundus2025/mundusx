# Qwen Image Edit worker

The separate `qwen-image-edit-2509-lightning-4step-832x480-v1` profile edits a supplied PNG using Qwen-Image-Edit-2509 and the pinned Lightning adapter. It does not change the text-to-image or Wan video profiles. The workflow uses 832×480, four steps, CFG 1, shift 3.1 and CFGNorm 1. Complete reference images are contained within the canvas rather than cropped. Generative edits can alter details even when preservation is requested.

A minimum 48 GiB contributed memory budget and a successful local edit verification are required. GX10 qualification used the managed Linux ARM64 ComfyUI runtime. This is not a 4 GiB GPU or macOS qualification.

With a configured contributor and sufficient contribution cap:

```sh
opengpu media --image-edit plan
opengpu media --image-edit setup --yes
opengpu media --image-edit verify --input-image reference.png
opengpu media --image-edit generate --input-image reference.png --prompt "Change the background to blue" --seed 42
```

Select `image-edit`/`image_edit` among contributor workloads before queue serving. Readiness is stored separately in `media/verified-edit-fast.json`. Missing or changed model hashes, reference checksums, contribution budgets or certificates block advertisement.

## Two services, one GPU slot

A media-only node can explicitly mirror to the public control plane while retaining its primary private connection:

```sh
export MUNDUSX_MEDIA_SERVER_URL=https://YOUR-PRIVATE-CHAT
export OPENGPU_MEDIA_SECONDARY_SERVER_URL=https://chat.mundusx.ai
export OPENGPU_SECONDARY_CONTROL_PLANE_URL=https://control.mundusx.ai
export OPENGPU_MEDIA_SECONDARY_PROFILES=qwen-image-edit-2509-lightning-4step-832x480-v1
opengpu-node-agent run
```

These are owner-side environment settings. Remote requests cannot choose new servers or local input paths. One worker lock and GPU job lock serialize both queues. Pending completion records retain their original server; completion retries cannot send a private result to another service. Existing endpoint-scoped gateway credentials are never applied to the public origin. The secondary heartbeat advertises only the configured subset; no LLM work is mirrored.

The helper handoff token takes precedence over the generic active-job marker so the node acknowledges the actual GPU handoff request.


## Two-image local edits

`workers/media/edit-with-reference.sh` is a local convenience launcher for the installed two-input CLI. On the media GX10 it is installed as `~/.opengpu/media/edit-with-reference.sh`, with shortcut `~/.local/bin/opengpu-edit-reference`.

```sh
opengpu-edit-reference /home/a/artifacts/TARGET.png /home/a/artifacts/REFERENCE.png "Apply the jacket from image 2 to the person in image 1."
```

The first file is the target scene; the second file is the reference. The script validates both PNG headers, prints the exact underlying command, and leaves source images intact. Add `--dry-run` before the paths for validation without model execution. Optional trailing arguments include `--seed 42` or slower `--quality --preserve-size`.
