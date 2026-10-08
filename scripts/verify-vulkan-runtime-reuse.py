#!/usr/bin/env python3
"""Refuse runtime reuse when the engine adapter changes; battery policy is independent."""
import re
import subprocess
import sys

def adapter(ref):
    source = subprocess.check_output(['git', 'show', f'{ref}:agents/node/src/worker.rs']).decode()
    # This contiguous block only controls battery/health admission, not engine execution.
    source, count = re.subn(r'(?ms)^(?:fn battery_contribution_reason\(.*?\n)?pub fn probe_worker_policy\(.*?(?=^fn extract_llama_response\()', '', source)
    if count != 1:
        raise SystemExit('Cannot identify independent policy block; rebuild Vulkan runtime')
    for name in ['battery_contribution_requires_charge_above_28_percent', 'battery_policy_allows_configured_caps_and_preserves_health_checks']:
        source = re.sub(r'(?ms)^    #\[test\]\n    fn ' + name + r'\(.*?(?=^    #\[test\])', '', source)
    return source

if len(sys.argv) != 3:
    raise SystemExit('usage: verify-vulkan-runtime-reuse.py OLD_REF NEW_REF')
if adapter(sys.argv[1]) != adapter(sys.argv[2]):
    raise SystemExit('Vulkan adapter changed outside battery policy; rebuild runtime')
print('Vulkan adapter unchanged outside independent battery policy')
