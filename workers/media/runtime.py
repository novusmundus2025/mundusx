"""OpenGPU media setup and bounded ComfyUI execution. Standard library only.

This helper is embedded in the CLI/agent. GPU libraries stay in an isolated
managed runtime; existing ComfyUI installations are never modified or stopped.
"""
import argparse
import contextlib
import hashlib
from fractions import Fraction
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import struct
import subprocess
import sys
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zlib

MAX_JSON = 8 * 1024 * 1024
MAX_IMAGE = 32 * 1024 * 1024
MAX_VIDEO = 128 * 1024 * 1024


def is_video(profile):
    return profile.get('operation') == 'text_to_video'


def profile_record(root, profile, name):
    return root / (name + ('-video' if is_video(profile) else '') + (('-' + str(profile['frames']) + 'f') if name == 'verified' and is_video(profile) and profile['frames'] != profile['fps'] * 2 + 1 else '') + '.json')


class MediaError(Exception):
    pass


def emit(event, **fields):
    if os.environ.get('OPENGPU_MEDIA_HUMAN_PROGRESS') == '1':
        if event == 'download_progress':
            return
        if event in ('checking_cache', 'cached', 'download', 'download_verified'):
            labels = {'checking_cache': 'Checking cached file', 'cached': 'Reusing verified file',
                      'download': 'Downloading', 'download_verified': 'Download checksum verified'}
            print(f"{labels[event]}: {fields['file']}", file=sys.stderr, flush=True)
            return
        if event == 'progress':
            print(f"Generating media... {fields.get('elapsed_seconds', 0)}s elapsed",
                  file=sys.stderr, flush=True)
            return
    print(json.dumps({"event": event, **fields}), flush=True)


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    with tmp.open("w", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(tmp, path)


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def digest_file(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def profile_hash(profile):
    return hashlib.sha256(json.dumps(profile, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def endpoint_url(value):
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise MediaError("Use an HTTP(S) ComfyUI endpoint without credentials, query, or fragment")
    try:
        parsed.port
    except ValueError as error:
        raise MediaError("Invalid endpoint port") from error
    if any(c.isspace() for c in value):
        raise MediaError("Invalid whitespace in endpoint")
    return value.rstrip("/")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class Comfy:
    def __init__(self, endpoint):
        self.endpoint = endpoint_url(endpoint)
        self.opener = urllib.request.build_opener(NoRedirect)

    def request(self, route, data=None, raw=False, raw_limit=MAX_IMAGE):
        body = None if data is None else json.dumps(data).encode()
        request = urllib.request.Request(self.endpoint + route, data=body,
                                         headers={"Content-Type": "application/json"})
        try:
            with self.opener.open(request, timeout=15) as response:
                limit = raw_limit if raw else MAX_JSON
                result = response.read(limit + 1)
                if len(result) > limit:
                    raise MediaError("ComfyUI response exceeds size limit")
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            raise MediaError("ComfyUI request failed at " + route.split('?')[0]) from error
        if raw:
            return result
        try:
            return json.loads(result)
        except (ValueError, UnicodeError) as error:
            raise MediaError("ComfyUI returned invalid JSON") from error

    def inspect(self, profile):
        stats = self.request("/system_stats")
        info = self.request("/object_info")
        if not isinstance(stats.get("system"), dict) or not isinstance(stats.get("devices"), list):
            raise MediaError("Endpoint does not report ComfyUI system metadata")
        required = {node["class_type"] for node in workflow(profile, "test", 1, "probe").values()}
        missing = sorted(required - set(info))
        if missing:
            raise MediaError("Missing ComfyUI nodes: " + ", ".join(missing))
        models = [("UNETLoader", "unet_name", 0), ("CLIPLoader", "clip_name", 1), ("VAELoader", "vae_name", 2)]
        if profile.get('architecture') == 'wan22_t2v_a14b':
            models.append(("UNETLoader", "unet_name", 3))
        for node, field, index in models:
            name = Path(profile["files"][index]["path"]).name
            choices = info[node].get("input", {}).get("required", {}).get(field, [[]])[0]
            if not isinstance(choices, list) or name not in choices:
                raise MediaError("Missing model in existing ComfyUI: " + name)
        return stats, info

    def ensure_idle(self):
        queue = self.request("/queue")
        if "queue_running" not in queue or "queue_pending" not in queue:
            raise MediaError("ComfyUI did not report queue state")
        if queue["queue_running"] or queue["queue_pending"]:
            raise MediaError("ComfyUI is busy; verification deferred until its queue is empty")


def workflow(profile, prompt, seed, prefix):
    if not prompt.strip() or len(prompt) > 16000:
        raise MediaError("Prompt must contain 1 to 16000 characters")
    if not isinstance(seed, int) or not 0 <= seed < 2**63:
        raise MediaError("Seed must be a nonnegative 63-bit integer")
    def node(kind, **inputs):
        return {"class_type": kind, "inputs": inputs}
    if is_video(profile):
        if profile.get('architecture') == 'wan22_t2v_a14b':
            common = dict(positive=["4", 0], negative=["5", 0], steps=profile["steps"],
                          cfg=profile["cfg"], sampler_name="euler", scheduler="simple")
            split = profile["switch_step"]
            if not 0 < split < profile["steps"]:
                raise MediaError("Invalid high/low noise sampler boundary")
            return {
                "1": node("UNETLoader", unet_name=Path(profile["files"][0]["path"]).name, weight_dtype="default"),
                "2": node("CLIPLoader", clip_name=Path(profile["files"][1]["path"]).name, type="wan", device="default"),
                "3": node("VAELoader", vae_name=Path(profile["files"][2]["path"]).name),
                "4": node("CLIPTextEncode", text=prompt, clip=["2", 0]),
                "5": node("CLIPTextEncode", text="blur, low quality, still frame, subtitles, watermark, distorted motion", clip=["2", 0]),
                "6": node("EmptyHunyuanLatentVideo", width=profile["width"], height=profile["height"], length=profile["frames"], batch_size=1),
                "7": node("ModelSamplingSD3", model=["1", 0], shift=8.0),
                "12": node("UNETLoader", unet_name=Path(profile["files"][3]["path"]).name, weight_dtype="default"),
                "13": node("ModelSamplingSD3", model=["12", 0], shift=8.0),
                "8": node("KSamplerAdvanced", model=["7", 0], latent_image=["6", 0], add_noise="enable",
                          noise_seed=seed, start_at_step=0, end_at_step=split, return_with_leftover_noise="enable", **common),
                "14": node("KSamplerAdvanced", model=["13", 0], latent_image=["8", 0], add_noise="disable",
                           noise_seed=0, start_at_step=split, end_at_step=profile["steps"], return_with_leftover_noise="disable", **common),
                "9": node("VAEDecode", samples=["14", 0], vae=["3", 0]),
                "11": node("CreateVideo", images=["9", 0], fps=profile["fps"]),
                "10": node("SaveVideo", video=["11", 0], filename_prefix="opengpu/" + prefix, format="auto", codec="h264"),
            }
        return {
            "1": node("UNETLoader", unet_name=Path(profile["files"][0]["path"]).name, weight_dtype="default"),
            "2": node("CLIPLoader", clip_name=Path(profile["files"][1]["path"]).name, type="wan", device="default"),
            "3": node("VAELoader", vae_name=Path(profile["files"][2]["path"]).name),
            "4": node("CLIPTextEncode", text=prompt, clip=["2", 0]),
            "5": node("CLIPTextEncode", text="blur, low quality, still frame, subtitles, watermark, distorted motion", clip=["2", 0]),
            "6": node("Wan22ImageToVideoLatent", vae=["3", 0], width=profile["width"], height=profile["height"], length=profile["frames"], batch_size=1),
            "7": node("ModelSamplingSD3", model=["1", 0], shift=8.0),
            "8": node("KSampler", model=["7", 0], positive=["4", 0], negative=["5", 0], latent_image=["6", 0],
                      seed=seed, steps=profile["steps"], cfg=profile["cfg"], sampler_name="uni_pc", scheduler="simple", denoise=1.0),
            "9": node("VAEDecode", samples=["8", 0], vae=["3", 0]),
            "11": node("CreateVideo", images=["9", 0], fps=profile["fps"]),
            # Pinned ComfyUI's explicit format path assumes an enum; AUTO produces MP4.
            # Validate H.264/MP4 after export instead of relying on the output extension.
            "10": node("SaveVideo", video=["11", 0], filename_prefix="opengpu/" + prefix, format="auto", codec="h264"),
        }
    if profile.get('operation') != 'text_to_image':
        raise MediaError('Unsupported media operation')
    return {
        "1": node("UNETLoader", unet_name=Path(profile["files"][0]["path"]).name, weight_dtype="default"),
        "2": node("CLIPLoader", clip_name=Path(profile["files"][1]["path"]).name, type="qwen_image", device="default"),
        "3": node("VAELoader", vae_name=Path(profile["files"][2]["path"]).name),
        "4": node("CLIPTextEncode", text=prompt, clip=["2", 0]),
        "5": node("CLIPTextEncode", text="", clip=["2", 0]),
        "6": node("EmptySD3LatentImage", width=profile["width"], height=profile["height"], batch_size=1),
        "7": node("ModelSamplingAuraFlow", model=["1", 0], shift=3.1),
        "8": node("KSampler", model=["7", 0], positive=["4", 0], negative=["5", 0], latent_image=["6", 0],
                  seed=seed, steps=profile["steps"], cfg=profile["cfg"], sampler_name="euler", scheduler="simple", denoise=1.0),
        "9": node("VAEDecode", samples=["8", 0], vae=["3", 0]),
        "10": node("SaveImage", images=["9", 0], filename_prefix="opengpu/" + prefix),
    }


def validate_png(data, width, height):
    """Validate checksums, dimensions, and complete bounded PNG scanline data."""
    if not data.startswith(b"\x89PNG\r\n\x1a\n") or len(data) > MAX_IMAGE:
        raise MediaError("Output is not a bounded PNG image")
    offset, header, compressed, ended = 8, None, bytearray(), False
    while offset + 12 <= len(data):
        size = struct.unpack(">I", data[offset:offset+4])[0]
        kind = data[offset+4:offset+8]
        end = offset + 12 + size
        if end > len(data):
            raise MediaError("Truncated PNG chunk")
        content = data[offset+8:end-4]
        if zlib.crc32(kind + content) & 0xffffffff != struct.unpack(">I", data[end-4:end])[0]:
            raise MediaError("PNG checksum mismatch")
        if kind == b"IHDR":
            if header is not None or offset != 8 or size != 13:
                raise MediaError("Invalid PNG header")
            header = struct.unpack(">IIBBBBB", content)
        elif kind == b"IDAT":
            compressed.extend(content)
        elif kind == b"IEND":
            ended = size == 0 and end == len(data)
            break
        offset = end
    if not ended or header is None or header[:2] != (width, height):
        raise MediaError("Output dimensions or PNG structure do not match the profile")
    _, _, depth, color, compression, filtering, interlace = header
    if depth != 8 or color not in (2, 6) or (compression, filtering, interlace) != (0, 0, 0):
        raise MediaError("Expected non-interlaced 8-bit RGB/RGBA PNG")
    row = width * (3 if color == 2 else 4) + 1
    expected = row * height
    decoder = zlib.decompressobj()
    try:
        pixels = decoder.decompress(compressed, expected + 1)
    except zlib.error as error:
        raise MediaError("Invalid PNG pixel data") from error
    if len(pixels) != expected or not decoder.eof or decoder.unused_data or any(pixels[i] > 4 for i in range(0, len(pixels), row)):
        raise MediaError("Invalid or oversized PNG scanline data")


def validate_video(path, profile):
    if not shutil.which('ffprobe') or not shutil.which('ffmpeg'):
        raise MediaError('Video verification requires ffprobe and ffmpeg on PATH')
    if path.stat().st_size > MAX_VIDEO:
        raise MediaError('Video exceeds the bounded MP4 size')
    result = subprocess.run(['ffprobe', '-v', 'error', '-max_alloc', '268435456', '-protocol_whitelist', 'file', '-format_whitelist', 'mov',
        '-show_streams', '-show_format', '-of', 'json', str(path)],
        capture_output=True, text=True, timeout=120)
    if result.returncode or len(result.stdout) > MAX_JSON:
        raise MediaError('Invalid MP4 output')
    info = json.loads(result.stdout)
    streams = info.get('streams', [])
    if len(streams) != 1:
        raise MediaError('Expected one silent video stream')
    stream = streams[0]
    if (stream.get('codec_type') != 'video' or stream.get('codec_name') != 'h264' or
        stream.get('pix_fmt') != 'yuv420p' or stream.get('width') != profile['width'] or
        stream.get('height') != profile['height'] or
        Fraction(stream.get('avg_frame_rate', '0')) != profile['fps'] or
        abs(float(info.get('format', {}).get('duration', 0)) - profile['frames']/profile['fps']) > 0.05):
        raise MediaError('Video dimensions, codec, frame count or duration do not match the profile')
    # Check stream bounds before asking either tool to decode untrusted output.
    counted = subprocess.run(['ffprobe', '-v', 'error', '-max_alloc', '268435456', '-protocol_whitelist', 'file',
        '-format_whitelist', 'mov', '-count_frames', '-select_streams', 'v:0', '-show_entries', 'stream=nb_read_frames',
        '-of', 'json', str(path)], capture_output=True, text=True, timeout=120)
    if counted.returncode or int(json.loads(counted.stdout)['streams'][0].get('nb_read_frames', 0)) != profile['frames']:
        raise MediaError('Video decoded frame count does not match the profile')
    decoded = subprocess.run(['ffmpeg', '-v', 'error', '-max_alloc', '268435456', '-xerror', '-nostdin', '-protocol_whitelist', 'file',
        '-format_whitelist', 'mov', '-i', str(path), '-map', '0:v:0', '-f', 'null', '-'],
        capture_output=True, timeout=120)
    if decoded.returncode:
        raise MediaError('Video does not completely decode')


def generate(client, profile, prompt, seed, output, timeout=None):
    if is_video(profile) and (not shutil.which('ffprobe') or not shutil.which('ffmpeg')):
        raise MediaError('Video generation requires ffprobe and ffmpeg on PATH for validation')
    client.inspect(profile)
    client.ensure_idle()
    token = uuid.uuid4().hex
    result = client.request("/prompt", {"prompt": workflow(profile, prompt, seed, token), "client_id": token})
    prompt_id = result.get("prompt_id")
    if not isinstance(prompt_id, str) or not prompt_id or result.get("node_errors"):
        raise MediaError("ComfyUI rejected the bundled workflow: " + json.dumps(result.get("node_errors", {}))[:2000])
    emit("submitted", prompt_id=prompt_id)
    started = time.monotonic()
    timeout = profile["timeout_seconds"] if timeout is None else timeout
    try:
        while time.monotonic() - started < timeout:
            entry = client.request("/history/" + urllib.parse.quote(prompt_id, safe="")).get(prompt_id)
            if entry:
                status = entry.get("status", {})
                if status.get("status_str") == "error":
                    raise MediaError("ComfyUI execution failed: " + json.dumps(status.get("messages", []))[:2000])
                if status.get("completed"):
                    images = entry.get("outputs", {}).get("10", {}).get("images", [])
                    if len(images) != 1 or images[0].get("type") != "output":
                        raise MediaError("Workflow did not produce exactly one output artifact")
                    image = images[0]
                    filename = image.get("filename", "")
                    subfolder = image.get("subfolder", "")
                    if not filename.startswith(token) or '/' in filename or '\\' in filename or subfolder != "opengpu":
                        raise MediaError("Unexpected output artifact identity")
                    query = urllib.parse.urlencode({"filename": filename, "subfolder": subfolder, "type": "output"})
                    data = client.request("/view?" + query, raw=True, raw_limit=MAX_VIDEO if is_video(profile) else MAX_IMAGE)
                    output.parent.mkdir(parents=True, exist_ok=True)
                    if is_video(profile):
                        if not filename.endswith('.mp4'):
                            raise MediaError('Expected an MP4 artifact')
                        temporary = output.with_name(output.name + '.' + uuid.uuid4().hex + '.partial.mp4')
                        try:
                            with temporary.open('xb') as stream: stream.write(data)
                            validate_video(temporary, profile)
                            # Never overwrite a pre-existing user output.
                            with output.open('xb') as stream: stream.write(data)
                        finally:
                            temporary.unlink(missing_ok=True)
                    else:
                        validate_png(data, profile["width"], profile["height"])
                        with output.open("xb") as stream: stream.write(data)
                    return {"artifact": str(output), "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
                            "duration_ms": round((time.monotonic()-started)*1000), "prompt_id": prompt_id}
            emit("progress", prompt_id=prompt_id, elapsed_seconds=round(time.monotonic()-started))
            time.sleep(2)
        raise MediaError("Generation timed out; running external ComfyUI work was not interrupted")
    except BaseException:
        # Delete only our pending item. Never call ComfyUI's global interrupt API.
        try:
            client.request("/queue", {"delete": [prompt_id]})
        except MediaError:
            pass
        raise


@contextlib.contextmanager
def lock(root):
    root.mkdir(parents=True, exist_ok=True)
    path = root / "media.lock"
    stream = path.open("a+b")
    try:
        if os.name == "nt":
            import msvcrt
            stream.seek(0); stream.write(b"0"); stream.flush(); stream.seek(0)
            try: msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            except OSError as error: raise MediaError("Another media operation is running") from error
        else:
            import fcntl
            try: fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError as error: raise MediaError("Another media operation is running") from error
        yield
    finally:
        stream.close()


def run(args, timeout=3600, capture=False):
    emit("command", program=args[0], action=args[1] if len(args)>1 else "")
    result = subprocess.run(args, timeout=timeout, stdout=subprocess.PIPE if capture else sys.stderr,
                            stderr=sys.stderr, text=True)
    if result.returncode:
        raise MediaError("Command failed: " + args[0] + " " + args[1])
    return result.stdout.strip() if capture else ""


def download(url, path, size, sha256):
    path.parent.mkdir(parents=True, exist_ok=True)
    emit("checking_cache", file=path.name)
    if path.exists() and path.stat().st_size == size and digest_file(path) == sha256:
        emit("cached", file=path.name, bytes=size)
        return
    partial = path.with_suffix(path.suffix + ".partial")
    emit("download", file=path.name, bytes=size)
    started = last_update = time.monotonic()
    with urllib.request.urlopen(url, timeout=60) as response, partial.open("wb") as stream:
        digest, count = hashlib.sha256(), 0
        while True:
            block = response.read(4*1024*1024)
            if not block: break
            count += len(block)
            if count > size: raise MediaError("Download exceeds expected size")
            stream.write(block); digest.update(block)
            now = time.monotonic()
            if now - last_update >= 1 or count == size:
                emit("download_progress", file=path.name, downloaded_bytes=count,
                     total_bytes=size, elapsed_seconds=round(now-started, 1))
                if sys.stderr.isatty():
                    fraction = min(count / max(size, 1), 1)
                    filled = int(fraction * 24)
                    print(f"\r{path.name}: [{'#'*filled}{'-'*(24-filled)}] "
                          f"{fraction:.0%} {count/1024**3:.2f}/{size/1024**3:.2f} GiB",
                          end="\n" if count == size else "", file=sys.stderr, flush=True)
                last_update = now
    if count != size or digest.hexdigest() != sha256:
        raise MediaError("Download checksum or size mismatch for " + path.name)
    os.replace(partial, path)
    emit("download_verified", file=path.name, bytes=count)


def cached_model(root, file):
    path = root / "models" / file["path"]
    emit("checking_cache", file=path.name)
    return path.is_file() and path.stat().st_size == file["size"] and digest_file(path) == file["sha256"]


def install(root, profile, budget_bytes):
    if platform.system() != "Linux" or platform.machine().lower() not in ('aarch64', 'arm64'):
        raise MediaError("Automatic media runtime currently targets Linux ARM64 GB10/GX10; connect existing ComfyUI on other platforms")
    if budget_bytes < profile["minimum_budget_bytes"]:
        raise MediaError(f"Selected profile requires at least {profile['minimum_budget_bytes']/1024**3:g} GiB contribution budget")
    if not shutil.which("docker") or not shutil.which("nvidia-smi"):
        raise MediaError("Automatic setup requires Docker, NVIDIA GPU drivers, and NVIDIA Container Toolkit")
    run(["docker", "info"], timeout=30, capture=True)
    gpu = run(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"], timeout=30, capture=True)
    if 'GB10' not in gpu or len(gpu.splitlines()) != 1:
        raise MediaError("Managed profile requires one GB10 GPU; use an existing ComfyUI endpoint for other hardware")
    # Existing valid files are reused, so only reserve space for missing/replaced files.
    total = sum(file["size"] for file in profile["files"]
                if not cached_model(root, file))
    if shutil.disk_usage(root).free < total + 15*1024**3:
        raise MediaError("Insufficient disk space: models plus 15 GiB runtime/build headroom required")
    revision = profile["comfy_revision"]
    archive = root / (revision + ".tar.gz")
    download("https://codeload.github.com/Comfy-Org/ComfyUI/tar.gz/" + revision, archive, 6829774, profile["comfy_archive_sha256"])
    build = root / ("build-" + revision)
    build.mkdir(exist_ok=True)
    with tarfile.open(archive) as tar:
        base = build.resolve()
        for member in tar.getmembers():
            target = (build / member.name).resolve()
            if not target.is_relative_to(base) or member.issym() or member.islnk() or not (member.isfile() or member.isdir()):
                raise MediaError("Unsafe entry in runtime source archive")
        tar.extractall(build, filter="data")
    source = build / ("ComfyUI-" + revision)
    run(["docker", "pull", profile["container_base"]])
    digest = json.loads(run(["docker", "image", "inspect", profile["container_base"], "--format", "{{json .RepoDigests}}"], capture=True))[0]
    # Existing NGC torch/torchvision stay pinned by the base image's constraints.
    requirements = (source / "requirements.txt").read_text()
    requirements = '\n'.join(line for line in requirements.splitlines() if not line.strip().startswith(("torch", "torchsde")))
    (source / "opengpu-requirements.txt").write_text(requirements + '\ntorchsde==0.2.6\n')
    dockerfile = f"FROM {digest}\nWORKDIR /opt/comfy\nCOPY . /opt/comfy\nRUN python -m pip install -r opengpu-requirements.txt\nENTRYPOINT [\"python\", \"main.py\"]\n"
    (source / "Dockerfile.opengpu").write_text(dockerfile)
    image = "opengpu-media:" + profile_hash(profile)[:16]
    run(["docker", "build", "-f", str(source / "Dockerfile.opengpu"), "-t", image, str(source)])
    image_id = run(["docker", "image", "inspect", image, "--format", "{{.Id}}"], capture=True)
    with drain_contributor(root.parent):
        run(["docker", "run", "--rm", "--gpus", "all", "--entrypoint", "python", image_id, "-c",
             "import torch; assert torch.cuda.is_available(); x=torch.ones(16,device='cuda'); assert x.sum().item()==16; print(torch.cuda.get_device_name())"], timeout=120)
    for file in profile["files"]:
        url = f"https://huggingface.co/{profile['models_repo']}/resolve/{profile['models_revision']}/split_files/{file['path']}"
        download(url, root / "models" / file["path"], file["size"], file["sha256"])
    record = {"kind": "managed_comfyui", "image_id": image_id, "base_digest": digest,
              "profile_hash": profile_hash(profile), "budget_bytes": budget_bytes, "installed_at": int(time.time())}
    atomic_json(profile_record(root, profile, 'runtime'), record)
    emit("installed", runtime=record)


@contextlib.contextmanager
def drain_contributor(home, timeout=600):
    config = load_json(home / "config.json") if (home / "config.json").exists() else {}
    root = home / 'media'
    request = root / 'drain-request.json'
    token = uuid.uuid4().hex
    atomic_json(request, {"id":token})
    try:
        if config.get('connected') and not config.get('paused'):
            emit('waiting_for_llm', timeout_seconds=timeout)
            deadline = time.monotonic()+timeout
            while True:
                try: ack = load_json(root/'drain-ack.json')
                except (OSError, ValueError): ack = {}
                if ack.get('id') == token and ack.get('drained') is True: break
                if time.monotonic() >= deadline:
                    raise MediaError("LLM drain was not acknowledged; update the node agent or pause contribution, let jobs finish, then retry media verify")
                time.sleep(1)
        else:
            state = load_json(home/'agent-state.json') if (home/'agent-state.json').exists() else {}
            if state.get('agent_state') == 'busy' or (home/'persistent-runtime.json').exists():
                raise MediaError("LLM runtime has not released resources yet; retry after contribution stops")
        yield
    finally:
        try:
            if load_json(request).get('id') == token: request.unlink()
        except (OSError, ValueError): pass


@contextlib.contextmanager
def client_for(root, profile, endpoint, budget_bytes):
    if endpoint:
        yield Comfy(endpoint)
        return
    record = load_json(profile_record(root, profile, 'runtime'))
    if record.get("profile_hash") != profile_hash(profile):
        raise MediaError("Managed runtime profile changed; run media setup again")
    # Only our new container is stopped. No global stop, external unload or prune.
    name = "opengpu-media-" + uuid.uuid4().hex
    outputs = root / "outputs"; outputs.mkdir(exist_ok=True)
    reserve_gb = max(0, (physical_memory() - budget_bytes) / 1024**3)
    owner = hashlib.sha256(str(root.resolve()).encode()).hexdigest()
    args = ["docker", "run", "--detach", "--rm", "--name", name, "--gpus", "all",
            "--label", "opengpu.media.owner=" + owner,
            "--publish", "127.0.0.1::8188", "--memory", str(budget_bytes),
            "--volume", f"{root / 'models'}:/opt/comfy/models:ro",
            "--volume", f"{outputs}:/opt/comfy/output", record["image_id"],
            "--listen", "0.0.0.0", "--port", "8188", "--disable-all-custom-nodes",
            "--reserve-vram", str(round(reserve_gb, 2))]
    atomic_json(root/'active-container.json', {"name": name, "owner": owner})
    try:
        run(args, timeout=120, capture=True)
        bindings = json.loads(run(["docker", "inspect", name, "--format", "{{json .NetworkSettings.Ports}}"], capture=True))
        port = bindings["8188/tcp"][0]["HostPort"]
        client = Comfy("http://127.0.0.1:" + port)
        deadline = time.monotonic() + 180
        while True:
            try:
                client.request("/system_stats")
                break
            except MediaError:
                if time.monotonic() >= deadline:
                    raise MediaError("Managed ComfyUI did not become ready within 180 seconds")
                time.sleep(2)
        yield client
    finally:
        stop_owned_container(root)


def stop_owned_container(root):
    marker = root/'active-container.json'
    if not marker.exists(): return
    record = load_json(marker)
    name, owner = record['name'], record['owner']
    suffix = name.removeprefix('opengpu-media-')
    expected = hashlib.sha256(str(root.resolve()).encode()).hexdigest()
    if not name.startswith('opengpu-media-') or len(suffix) != 32 or any(c not in '0123456789abcdef' for c in suffix) or owner != expected:
        raise MediaError('Invalid owned-container marker; refusing container cleanup')
    # Listing must succeed: daemon failure is not evidence the container stopped.
    names = run(['docker','ps','--all','--format','{{.Names}}'], capture=True).splitlines()
    if name in names:
        actual = run(['docker','inspect',name,'--format','{{index .Config.Labels "opengpu.media.owner"}}'], capture=True)
        if actual != owner: raise MediaError('Container ownership mismatch; refusing cleanup')
        run(['docker','stop','--time','10',name], timeout=30, capture=True)
    marker.unlink()


def physical_memory():
    if platform.system() == "Linux":
        for line in Path('/proc/meminfo').read_text().splitlines():
            if line.startswith('MemTotal:'): return int(line.split()[1])*1024
    if platform.system() == "Darwin":
        return int(subprocess.check_output(['sysctl','-n','hw.memsize'], text=True))
    if os.name == "nt":
        import ctypes
        class Status(ctypes.Structure):
            _fields_ = [('length',ctypes.c_ulong),('load',ctypes.c_ulong)] + [(name,ctypes.c_ulonglong) for name in ['total','available','page','available_page','virtual','available_virtual','extended']]
        status=Status(); status.length=ctypes.sizeof(status)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)): return status.total
    raise MediaError("Cannot determine machine memory")


def preset_profile(profile, seconds):
    if seconds not in (2, 5, 10):
        raise MediaError('Video seconds must be 2, 5 or 10')
    if not is_video(profile) or seconds == 2:
        return profile
    frames = seconds * profile['fps'] + 1
    return {**profile, 'frames': frames, 'id': profile['id'].replace(f"-{profile['frames']}f-", f'-{frames}f-'),
            'timeout_seconds': max(profile['timeout_seconds'], 7200)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['plan','setup','verify','generate','status','stop','upload'])
    parser.add_argument('--home', required=True, type=Path)
    parser.add_argument('--profile', required=True, type=Path)
    parser.add_argument('--endpoint')
    parser.add_argument('--cap-percent', type=int, default=70)
    parser.add_argument('--yes', action='store_true')
    parser.add_argument('--prompt')
    parser.add_argument('--seed', type=int, default=42)
    parser.add_argument('--file', type=Path)
    parser.add_argument('--ticket', type=Path)
    parser.add_argument('--server')
    parser.add_argument('--video', action='store_true')
    parser.add_argument('--seconds', type=int, choices=[2, 5, 10], default=2)
    args = parser.parse_args()
    if args.action == 'upload':
        from artifact_upload import upload_file
        emit('uploaded', **upload_file(args.file, args.ticket, args.server))
        return
    profile = load_json(args.profile)
    base_profile = profile
    profile = preset_profile(base_profile, args.seconds)
    if args.prompt is None:
        args.prompt = ('A red ceramic teapot slowly rotates on a wooden table, soft daylight, steady camera'
                       if is_video(profile) else 'A red ceramic teapot on a wooden table, soft daylight, photorealistic')
    root = args.home.resolve() / 'media'
    if not 1 <= args.cap_percent <= 100: raise MediaError("Invalid contribution percentage")
    budget = physical_memory()*args.cap_percent//100
    plan = {"profile": profile['id'], "license": profile['license'], "license_url": profile['license_url'],
            "model_download_bytes": sum(f['size'] for f in profile['files']), "budget_bytes": budget,
            "automatic_platform_supported": platform.system() == 'Linux' and platform.machine().lower() in ('aarch64','arm64'),
            "operation": profile['operation'], "width": profile['width'], "height": profile['height'],
            "frames": profile.get('frames'), "fps": profile.get('fps'), "steps": profile['steps'],
            "local_video_tools_available": bool(shutil.which('ffprobe') and shutil.which('ffmpeg')) if is_video(profile) else None,
            "network_dispatch_enabled": False}
    if args.action == 'plan': emit('plan', **plan); return
    if args.action == 'status':
        record = profile_record(root, profile, 'verified')
        certificate = load_json(record) if record.exists() else None
        if certificate and (certificate.get('profile_hash') != profile_hash(profile) or
                            certificate.get('cap_percent') != args.cap_percent or
                            certificate.get('endpoint') != args.endpoint or
                            (root/'active-container.json').exists()):
            certificate['ready'] = False
            certificate['reason'] = 'Configuration changed or container cleanup is pending; verify again'
        emit('status', verified=certificate, network_dispatch_enabled=False); return
    with lock(root):
        stop_owned_container(root)
        if args.action == 'stop': emit('stopped'); return
        if args.action == 'setup':
            emit('plan', **plan)
            if not args.yes: raise MediaError("Review the plan and rerun with --yes to download and install")
            if is_video(profile) and (not shutil.which('ffprobe') or not shutil.which('ffmpeg')):
                raise MediaError('Install ffmpeg and ffprobe on PATH before video setup')
            if args.endpoint:
                Comfy(args.endpoint).inspect(profile)
                emit('connected', endpoint=endpoint_url(args.endpoint))
            else:
                install(root, base_profile, budget)
            return
        if budget < profile['minimum_budget_bytes']: raise MediaError("Insufficient contribution budget for this profile")
        config = load_json(args.home/'config.json') if (args.home/'config.json').exists() else {}
        if args.endpoint and config.get('connected') and not config.get('paused'):
            raise MediaError('External ComfyUI can retain GPU memory: pause contribution and let LLM work stop before verification; release external model memory before resuming contribution')
        # Invalidate before testing so failure never leaves an old ready marker.
        atomic_json(profile_record(root, profile, 'verified'), {"ready": False, "profile_hash": profile_hash(profile)})
        with drain_contributor(args.home), client_for(root, base_profile, args.endpoint, budget) as client:
            stats, _ = client.inspect(profile)
            output = root/'artifacts'/(uuid.uuid4().hex + ('.mp4' if is_video(profile) else '.png'))
            result = generate(client, profile, args.prompt, args.seed, output)
            certificate = {"ready": True, "profile": profile['id'], "profile_hash": profile_hash(profile),
                           "operation": profile['operation'], "width": profile['width'], "height": profile['height'],
                           "steps": profile['steps'], "budget_bytes": budget, "cap_percent": args.cap_percent, "verified_at": int(time.time()),
                           "runtime": 'external' if args.endpoint else 'managed', "endpoint": args.endpoint,
                           "frames": profile.get('frames'), "fps": profile.get('fps'),
                           "devices": stats.get('devices', []), "network_dispatch_enabled": False, **result}
        atomic_json(profile_record(root, profile, 'verified'), certificate)
        emit('completed', **certificate)


if __name__ == '__main__':
    try:
        main()
    except (MediaError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        emit('failed', error=str(error))
        sys.exit(1)
