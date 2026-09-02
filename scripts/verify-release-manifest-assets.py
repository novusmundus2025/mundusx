#!/usr/bin/env python3
"""Fail a release when its manifest and packaged files disagree."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fail(message: str) -> None:
    print(f"FAILED: {message}", file=sys.stderr)
    raise SystemExit(1)


root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
manifest_path = root / "release-manifest.json"
if not manifest_path.is_file():
    fail(f"missing manifest: {manifest_path}")

manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
expected: list[tuple[str, str]] = [
    (manifest.get("binary_name", ""), manifest.get("checksum_sha256", ""))
]
for section in ("assets", "runtime_assets", "installers"):
    for item in manifest.get(section, []):
        expected.append((item.get("name", ""), item.get("checksum_sha256", "")))

for name, declared in expected:
    if not name or not declared:
        fail("manifest contains an asset without a name or checksum")
    artifact = root / name
    sidecar = root / f"{name}.sha256"
    if not artifact.is_file():
        fail(f"manifest references missing asset: {name}")
    if not sidecar.is_file():
        fail(f"missing checksum sidecar: {sidecar.name}")
    actual = sha256(artifact)
    sidecar_checksum = sidecar.read_text(encoding="ascii").split()[0].lower()
    if actual != declared.lower():
        fail(f"manifest checksum does not match {name}")
    if actual != sidecar_checksum:
        fail(f"checksum sidecar does not match {name}")
    print(f"verified {name}: {actual}")

print(f"verified all {len(expected)} manifest assets")
