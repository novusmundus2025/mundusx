# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

For the single working checklist, see [docs/master-checklist.md](/Users/DBATALL/Documents/aigrid/docs/master-checklist.md).

## Next Up

1. **Port secure device identity to all platforms**
   - macOS currently uses a file-encrypted, sign-only fallback
   - keep the private key non-exportable on Windows and Linux too
   - move macOS to OS-backed secure storage when the platform path is ready

2. **Define the federated governance model**
   - document the top-level standards / clearing-house org
   - document how operator companies join and certify
   - define settlement, revocation, and protocol versioning rules
   - keep the company control plane separate from the governance layer

3. **Public install and release rollout**
   - signed release binaries
   - public install endpoint
   - packaging checks for macOS and Linux release artifacts
   - Homebrew and WinGet publishing

## Why These Are Pending

- The Mac-first core runtime is now working end to end, so the remaining work is mostly platform expansion and productization.
- Secure device identity is sign-only and non-exportable on macOS in the current fallback path, but the other platforms still need the same treatment.
- Onboarding and governance are still design-heavy product layers rather than runtime plumbing.
- Release packaging is still required before public rollout.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.
