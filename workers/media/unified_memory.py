"""Managed ComfyUI entry point with GB10 unified-memory accounting.

CUDA free memory on GB10 can exclude reclaimable Linux file cache. Use the
OS available memory for this integrated GPU, preserving the cap/reserve and
all normal accounting on discrete GPUs. Only this owned process is patched.
"""
import runpy
import sys
import json
import os
from pathlib import Path
import threading
import time


def lease_valid(path, owner, now=None):
    try:
        value = json.loads(Path(path).read_text())
        return (isinstance(owner, str) and len(owner) == 32
                and value.get('owner') == owner
                and isinstance(value.get('expires_at'), (int, float))
                and value['expires_at'] > (time.time() if now is None else now))
    except (OSError, ValueError, TypeError):
        return False


def watch_lease(path, owner, stop, terminate=os._exit):
    while not stop.wait(5):
        if not lease_valid(path, owner):
            print('OpenGPU: serving lease expired; releasing managed runtime', flush=True)
            terminate(75)
            return


def install_gb10_memory_accounting(torch, psutil):
    original = torch.cuda.mem_get_info
    if not torch.cuda.is_available():
        return False
    if "GB10" not in torch.cuda.get_device_name(0):
        return False

    def mem_get_info(device=None):
        free, total = original(device)
        # Do not substitute unified memory for another device in a mixed system.
        if "GB10" not in torch.cuda.get_device_name(device):
            return free, total
        memory = psutil.virtual_memory()
        total = total or memory.total
        return min(total, max(free, memory.available)), total

    torch.cuda.mem_get_info = mem_get_info
    return True


if __name__ == "__main__":
    lease = os.environ.get('OPENGPU_MEDIA_LEASE_PATH')
    if lease:
        owner = os.environ.get('OPENGPU_MEDIA_OWNER_INSTANCE')
        if not lease_valid(lease, owner): sys.exit('OpenGPU: invalid serving lease')
        threading.Thread(target=watch_lease, args=(lease, owner, threading.Event()), daemon=True).start()
    import torch
    import psutil
    if install_gb10_memory_accounting(torch, psutil):
        print("OpenGPU: GB10 memory accounting includes reclaimable system cache", flush=True)
    sys.argv[0] = "/opt/comfy/main.py"
    sys.path.insert(0, "/opt/comfy")
    runpy.run_path("/opt/comfy/main.py", run_name="__main__")
