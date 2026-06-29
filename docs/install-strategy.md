# Install Strategy

This page defines the recommended way to distribute `opengpu` with native one-command installers that download a signed release binary and hand off first-run setup to `opengpu install`.

## Guiding Principle

- Ship a signed native binary.
- Keep Rust as the build tool, not a user dependency.
- Make the first install path one command wherever possible.

## Recommended Distribution Layers

### 1. Primary Channel

Use your own installer that downloads the correct release binary from the local release preview during development, then GitHub Releases later.

Recommended flow:

- macOS/Linux: the local install page at `http://127.0.0.1:<port>/install` should show `RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh`
- Windows: the local install page should show `.\install.ps1 -ReleaseBaseUrl http://127.0.0.1:8788/releases/latest/download -AllowUnsignedLocalPreview` only for unsigned local preview fixtures; production installs must omit the override.
- during localhost review, `scripts/local-release-preview.sh up` should build and serve the repo-managed preview that backs that command
- the same local page should expose a machine-readable manifest at `http://127.0.0.1:<port>/install.json`
- the same dashboard should also mirror the public-endpoint shape at `http://127.0.0.1:<port>/public/install` and `http://127.0.0.1:<port>/public/install.json`
- the matching localhost release preview should expose a machine-readable manifest at `http://127.0.0.1:8788/releases/latest/download/release-manifest.json`
- the repo-owned Pages artifact should mirror the public release/distribution surface at `/public/release` and `/public/release.json`

This should be the source of truth for release artifacts and checksums.

### Enterprise Windows Verification

The Windows PowerShell bootstrapper fails closed by default. A production install must provide:

- `opengpu-x86_64-pc-windows-msvc.exe`
- `opengpu-x86_64-pc-windows-msvc.exe.sha256`
- `release-manifest.json`
- `release-manifest.json.sig`

The installer verifies the asset checksum, requires the signed manifest artifacts, and checks that the manifest names the same Windows binary and checksum. `-AllowUnsignedLocalPreview` is reserved for local development fixtures and must not be used for enterprise or production installs.

The broader enterprise Windows rollout policy, including secret storage, model-source integrity, trusted runtime paths, rollback, and support boundaries, lives in [docs/enterprise-windows-policy.md](enterprise-windows-policy.md).

### Windows UAT Install Smoke

For UAT, Windows can be verified against a local preview release source before the public GitHub release asset is published. Run the repository smoke from a Windows x64 host:

```powershell
npm run test:windows-uat-install
```

The smoke builds the current `opengpu.exe` and `opengpu-node-agent.exe`, stages `opengpu-x86_64-pc-windows-msvc.exe` with a checksum and manifest in a temporary release directory, installs it through `install.ps1`, and uses an isolated `OPENGPU_HOME`. It then verifies:

- `opengpu install --private --control-plane-url http://127.0.0.1:8787 --cap-percent 30` writes the UAT/local control-plane URL
- `opengpu login` creates the Windows DPAPI-protected `operator-token.dpapi` blob
- `config.json` does not contain the plaintext operator token
- `opengpu doctor --json` reports the protected token as present
- `opengpu-node-agent health --json` returns health and policy payloads that operators can act on

This smoke does not replace the public Windows release asset work in [mundusx/mundusx#73](https://github.com/mundusx/mundusx/issues/73). It proves the UAT/local-preview install path while public distribution remains tracked separately.

### Machine Detection

The setup flow must detect the machine family before selecting runtime defaults or release assets:

- `macos-aarch64-apple-silicon` for Apple Silicon M-series nodes
- `windows-x86_64-cuda` for Windows NVIDIA CUDA contributors
- `linux-x86_64-cuda` and `linux-aarch64-cuda` for Linux NVIDIA CUDA contributors
- generic Windows/Linux/macOS profiles when no supported accelerator is detected

The same detection should drive:

- which binary asset the installer downloads: `opengpu-aarch64-apple-darwin`, `opengpu-x86_64-unknown-linux-gnu`, or `opengpu-x86_64-pc-windows-msvc.exe`
- whether CUDA, Apple Silicon, or generic setup instructions are shown
- which model catalog options are offered
- the model VRAM budget after applying the selected community contribution cap

The binary bootstrapper does only platform and asset detection. The CLI setup wizard owns control-plane selection, contribution cap, backend preference, and model gating so every install path reaches the same policy.

### 2. macOS Convenience

Add a Homebrew tap or formula for users who prefer `brew`.

The repo-managed local release preview now generates `homebrew/opengpu.rb` from the signed
release manifest so formula publishing can reuse the same artifact URL and checksum source.

Why:

- familiar on macOS
- easy to update
- good developer UX

### 3. Windows Convenience

Use `install.ps1` as the direct Windows path and add WinGet as the convenience channel for mainstream Windows distribution.

Why:

- built into modern Windows workflows
- good fit for CLI tooling
- easier for non-developer users than manual downloads

### 4. Linux Convenience

Keep the shell installer as the main Linux path.

Optional later:

- `.deb`
- `.rpm`

## What Not To Use

- `npm` is not a good fit for a native systems CLI.
- Cargo install should remain a developer-only path.
- Users should not need Rust, Node, or a manual toolchain setup.

## Release Matrix

Recommended release targets:

- `macos-aarch64`
- `macos-x86_64` later if Intel support becomes necessary
- `windows-x86_64`
- `linux-x86_64`
- `linux-aarch64` later if needed

## Windows Release Runner Decision

Windows CLI releases should build on GitHub-hosted `windows-latest` runners with the stable Rust MSVC toolchain target `x86_64-pc-windows-msvc`. This keeps the production `.exe` build on a native Windows linker and runtime instead of relying on cross-compilation for the first enterprise Windows channel.

The Windows matrix entry should produce:

- `opengpu-x86_64-pc-windows-msvc.exe`
- `opengpu-x86_64-pc-windows-msvc.exe.sha256`
- `release-manifest.json`
- `release-manifest.json.sig`

The Windows job should follow the same release workflow contract as the existing Linux and Apple Silicon jobs:

- install the stable Rust toolchain with the `x86_64-pc-windows-msvc` target
- build `apps/cli/Cargo.toml` in release mode for that target
- copy `target/x86_64-pc-windows-msvc/release/opengpu.exe` to `opengpu-x86_64-pc-windows-msvc.exe`
- generate and verify the SHA-256 checksum before upload
- run the repo packaging verifier against the Windows asset directory
- run `scripts/release-signing.sh prepare` with `OPENGPU_RELEASE_SIGNING_PRIVATE_KEY_PEM_B64` and `OPENGPU_RELEASE_SIGNING_PUBLIC_KEY_PEM_B64`
- upload the Windows `.exe`, `.sha256`, signed manifest, and manifest signature into the tagged GitHub release

Release signing secrets should stay shared across platform jobs and be provided only through GitHub Actions secrets. A Windows release job must fail if the signing secrets are missing or malformed; generated local preview keys are only acceptable for localhost preview fixtures and must not sign tagged release artifacts.

This decision unblocks the implementation issue for publishing `opengpu-x86_64-pc-windows-msvc.exe`: maintainers should add a `windows-latest` matrix row to `.github/workflows/release-cli.yml` rather than waiting on a separate runner/toolchain decision.

Track the remaining release-channel implementation work in:

- Windows release asset: [mundusx/mundusx#73](https://github.com/mundusx/mundusx/issues/73)
- Windows release runner and signing decision record: [mundusx/mundusx#109](https://github.com/mundusx/mundusx/issues/109)
- Homebrew channel publishing: [mundusx/mundusx#110](https://github.com/mundusx/mundusx/issues/110)
- WinGet package publishing: [mundusx/mundusx#111](https://github.com/mundusx/mundusx/issues/111)

## Release Checklist

### Shared Requirements

Before shipping any platform release:

- build from a clean tagged commit
- embed the CLI version in the binary
- generate checksums for every artifact
- validate the artifact shape with `scripts/verify-release-packaging.sh` and the localhost smoke test
- generate a machine-readable release verification report with `scripts/release-monitor-report.sh`
- verify checksums in CI before publishing the release assets
- sign the release manifest and publish the verified signature with the artifacts
- publish release notes with the exact tag
- verify the installer can fetch the matching asset
- verify the binary starts without extra dependencies
- verify `opengpu start` and `opengpu status` work after install

### macOS Checklist

Ship when all of these are true:

- `macos-aarch64` binary builds and runs on Apple Silicon
- the shell installer works on macOS
- the installer verifies release checksums when they are published
- the binary is notarized or otherwise signed according to release policy
- Homebrew tap or formula can install the same version
- `opengpu` is available in `PATH` after install

### Windows Checklist

Ship when all of these are true:

- `windows-x86_64` binary builds and runs on Windows
- PowerShell bootstrapper downloads the correct release asset
- PowerShell bootstrapper fails closed when checksum or signed manifest artifacts are missing
- installer handles `.exe` placement and PATH setup
- WinGet package installs the same release version
- binary starts from a normal PowerShell or Terminal session
- version output matches the tagged release

### Linux Checklist

Ship when all of these are true:

- `linux-x86_64` binary builds and runs on mainstream Linux
- shell installer works on common distros
- binary is executable after install
- optional `.deb` or `.rpm` packages are ready if you choose to publish them
- checksum verification works from the installer
- version output matches the tagged release

## Publishing Model

1. Build binaries in GitHub Actions.
2. Verify the generated checksums in CI.
3. Sign the release manifest for the tagged build.
4. Attach the verified artifacts and signed manifest to the tagged release.
5. Publish checksums and signatures.
6. Let the installer and package managers fetch from the release channel.

## Definition Of Done

The install strategy is complete when:

- users can install on macOS, Windows, and Linux without a manual toolchain
- each platform has one recommended path and one convenience path where possible
- release assets are signed, versioned, and documented
- the docs point to a single source of truth for downloads and update behavior
- the install path is simple enough that a new user can get to `opengpu start` in one step

## Product Positioning

- `opengpu` remains the CLI name.
- `mundusx.ai` remains a future public install entrypoint.
- The local install page should be the same command the installer docs use.
- GitHub Releases are the artifact source.
- The local docs site mirrors the install, identity, onboarding, credits, and release pages before anything is wired to the public domain.
