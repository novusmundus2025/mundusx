#!/usr/bin/env bash
set -euo pipefail
usage() {
  cat <<'HELP'
Usage: opengpu-edit-reference [--dry-run] TARGET.png REFERENCE.png "CHANGE" [EDIT OPTIONS]

TARGET.png is image 1: the image to change.
REFERENCE.png is image 2: the reference for the requested change.

Example:
  opengpu-edit-reference /home/a/artifacts/target.png /home/a/artifacts/reference.png "Use the jacket from image 2 on the person in image 1."

Optional edit options: --seed 42, --quality, --preserve-size, --steps 30.
Default: installed four-step image-edit profile. --quality uses slower standard sampling.
--dry-run validates both PNG files and prints the command without running the model.
Outputs are reported by OpenGPU under ~/.opengpu/media/artifacts.
HELP
}
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then usage; exit 0; fi
dry_run=false
if [[ "${1:-}" == "--dry-run" ]]; then dry_run=true; shift; fi
if (( $# < 3 )); then usage >&2; exit 2; fi
target=$1
reference=$2
change=$3
shift 3
if [[ -z "${change//[[:space:]]/}" ]]; then echo 'Describe the requested change.' >&2; exit 2; fi
python3 - "$target" "$reference" <<'PY'
import pathlib,struct,sys
for role,name in zip(('Target','Reference'),sys.argv[1:]):
 p=pathlib.Path(name).expanduser()
 if not p.is_file():raise SystemExit(f'{role} PNG does not exist: {p}')
 with p.open('rb') as f:h=f.read(24)
 if len(h)<24 or h[:8]!=b'\x89PNG\r\n\x1a\n' or h[12:16]!=b'IHDR':raise SystemExit(f'{role} must be a PNG: {p}')
 w,h=struct.unpack('>II',h[16:24])
 if not w or not h:raise SystemExit(f'{role} PNG has invalid dimensions')
 print(f'{role}: {p} ({w}x{h})')
PY
cli="$HOME/.local/bin/opengpu"
[[ -x "$cli" ]] || { echo "OpenGPU launcher missing: $cli" >&2; exit 2; }
prompt="Image 1 is the target image to edit. Image 2 is the reference image. Requested change: $change"
command=("$cli" media --image-edit --reference-image "$reference" generate --input-image "$target" --prompt "$prompt" "$@")
printf 'Command: '; printf '%q ' "${command[@]}"; printf '\n'
if $dry_run; then echo 'Dry run only; no model executed.'; exit 0; fi
exec "${command[@]}"
