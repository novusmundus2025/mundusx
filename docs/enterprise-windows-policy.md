# Enterprise Windows Install And Runtime Policy

This page is the operator review surface for enterprise Windows contributor deployment. It gathers the install trust, local secret storage, model source, runtime executable, rollback, and support-boundary requirements that must hold before a Windows host is treated as production-ready.

## Scope

This policy applies to Windows x64 contributor machines that install `opengpu`, run the CLI setup flow, and optionally run `opengpu-agent` for CUDA-backed work. Local preview and developer fixtures are allowed only when explicitly marked below.

## Install Trust

Enterprise installs must use the normal PowerShell bootstrapper path without local-preview overrides:

```powershell
.\install.ps1 -ReleaseBaseUrl <release-download-url>
```

A production release source must publish all of these artifacts together:

- `opengpu-x86_64-pc-windows-msvc.exe`
- `opengpu-x86_64-pc-windows-msvc.exe.sha256`
- `opengpu-node-agent-x86_64-pc-windows-msvc.exe`
- `opengpu-node-agent-x86_64-pc-windows-msvc.exe.sha256`
- `llama-runtime-x86_64-pc-windows-msvc-cuda.zip`
- `llama-runtime-x86_64-pc-windows-msvc-cuda.zip.sha256`
- `release-manifest.json`
- `release-manifest.json.sig`

The bootstrapper must fail closed when the checksum, manifest, signature, expected artifact kind, expected binary name, expected checksum, node-agent asset, CUDA runtime bundle, or CUDA runtime checksum is missing or mismatched. The signed release manifest must include an `assets` entry for the Windows node agent and a `runtime_assets` entry for the Windows CUDA llama.cpp runtime bundle before CUDA hosts are considered installable. `-AllowUnsignedLocalPreview` is reserved for unsigned localhost fixtures and must not be used for enterprise rollout, UAT against hosted releases, or production installs.

## Secret Storage

Windows contributor state must separate inspectable config from secret material:

- Device identity private-key material is stored in a DPAPI-protected identity record and is not persisted as raw `private_key_hex`.
- CLI operator bearer tokens are stored outside `config.json` in a DPAPI-protected blob.
- `config.json`, agent state, heartbeat logs, and model manifests must remain safe to inspect in support tickets.
- Existing plaintext Windows device identities must be rejected or migrated only through an explicit, documented path.

CLI and node-agent signing may decrypt protected key material locally only for the immediate signing operation. The public trust path reported to the control plane should distinguish Windows DPAPI-backed identity from legacy development storage.

## Model Sources

Official remote model downloads must be reviewable and integrity checked:

- Official presets live in `apps/cli/config/official-models.json`.
- Remote official catalog entries must include a non-empty SHA-256 before they are eligible for automatic download.
- The CLI must fail closed when an official remote model lacks a checksum or fails checksum verification.
- Local `file://` imports and test fixtures must stay explicit and separate from official remote presets.
- CUDA model eligibility must respect the selected community contribution cap and the cap-applied VRAM budget.

Operators should treat model catalog changes as release-risk changes because they affect downloaded third-party artifacts and scheduler-visible capability claims.

## Trusted Runtime Executables

Windows worker and diagnostic paths must not depend on unqualified `PATH` lookup in enterprise mode. Runtime setup should resolve and record trusted absolute paths for tools such as local model runners and GPU diagnostics, then validate those paths before use.

Minimum expectations:

- Worker launch uses a stored absolute executable path rather than `llama-cli` or similar bare command names.
- The Windows bootstrapper installs the signed node agent, extracts the signed CUDA runtime bundle, and records the extracted `llama-cli.exe` absolute path plus SHA-256 in `trusted-runtime-paths.json`.
- Diagnostics distinguish a missing runtime, an untrusted runtime, and a runtime execution failure.
- Runtime path changes are detected and reported before the node advertises ready capability.
- Publisher signature or hash validation should be used where practical for installed runtime tools.

Until trusted runtime path validation is complete, affected Windows nodes should advertise degraded or not-ready status instead of accepting jobs that require the untrusted runtime.

## Rollback, Uninstall, And Recovery

Rollback and uninstall must preserve operator control over identity and secrets:

- App binary uninstall alone must not silently delete the device identity.
- Reinstall should reuse the existing protected identity when the MundusX data directory remains intact.
- Explicit reset or revocation should be required to retire the device identity.
- A failed install or rollback must leave no partially trusted binary marked ready in PATH.
- Operators must be able to remove the protected operator token with `opengpu logout`.

If a Windows host is wiped, loses DPAPI-protected material, or deletes the MundusX data directory, it must enroll as a new device and the old device should be revoked from the control plane.

## Support Boundaries

Enterprise support should separate product issues from local environment issues:

- Missing NVIDIA drivers, missing CUDA runtime support, unreachable LM Studio, absent model cache, or low VRAM are host readiness issues until diagnostics show a product defect.
- Local preview commands, unsigned fixtures, and developer release sources are not production evidence.
- Hosted release installs require signed manifest artifacts and checksum verification before a host can be considered enterprise-ready.
- Human-readable support bundles may include config, health output, and logs, but must not include DPAPI-protected blobs, raw tokens, raw private keys, or copied model artifacts.

## Deployment Gate

A Windows contributor host is enterprise-ready only when all of these are true:

- The CLI was installed from a signed, checksum-verified release source.
- `opengpu install` completed without local-preview overrides.
- Device identity and operator tokens are protected outside plaintext JSON.
- Official remote model downloads are checksum-verified.
- Runtime executable paths are trusted or the node advertises not-ready status for affected job types.
- `opengpu doctor` and `opengpu-agent health` report no blocking install, identity, runtime, or policy failures.
