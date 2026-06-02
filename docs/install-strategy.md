# Install Strategy

This page defines the recommended way to distribute `opengpu`, starting with the Mac-first localhost install page and binary installer, then expanding later.

## Guiding Principle

- Ship a signed native binary.
- Keep Rust as the build tool, not a user dependency.
- Make the first install path one command wherever possible.

## Recommended Distribution Layers

### 1. Primary Channel

Use your own installer that downloads the correct release binary from the local release preview during development, then GitHub Releases later.

Recommended flow:

- macOS (current focus): the local install page at `http://127.0.0.1:<port>/install` should show `RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh`
- during localhost review, `scripts/local-release-preview.sh up` should build and serve the repo-managed preview that backs that command
- the same local page should expose a machine-readable manifest at `http://127.0.0.1:<port>/install.json`
- the same dashboard should also mirror the public-endpoint shape at `http://127.0.0.1:<port>/public/install` and `http://127.0.0.1:<port>/public/install.json`
- the matching localhost release preview should expose a machine-readable manifest at `http://127.0.0.1:8788/releases/latest/download/release-manifest.json`
- Linux and Windows: follow later, once the Mac release path is stable

This should be the source of truth for release artifacts and checksums.

### 2. macOS Convenience

Add a Homebrew tap or formula for users who prefer `brew`.

Why:

- familiar on macOS
- easy to update
- good developer UX

### 3. Windows Convenience

Add WinGet support first for mainstream Windows distribution.

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
- `windows-x86_64` later
- `linux-x86_64` later
- `linux-aarch64` later if needed

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
- `novusx.ai` remains a future public install entrypoint.
- The local install page should be the same command the installer docs use.
- GitHub Releases are the artifact source.
- The local docs site mirrors the install, identity, onboarding, credits, and release pages before anything is wired to the public domain.
