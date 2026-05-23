# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

## Next Up

1. **Port secure device identity to all platforms**
   - macOS secure storage is in place
   - keep the private key non-exportable on Windows and Linux too
   - preserve the current file-backed prototype only as a dev fallback

2. **Polish contributor onboarding**
   - show the secure device identity and hostname clearly on first run
   - explain the contribution cap and quiet policy
   - make the dashboard and CLI tell a consistent story

3. **Define the federated governance model**
   - document the top-level standards / clearing-house org
   - document how operator companies join and certify
   - define settlement, revocation, and protocol versioning rules
   - keep the company control plane separate from the governance layer

4. **Public install and release rollout**
   - signed release binaries
   - public install endpoint
   - packaging checks for macOS and Linux release artifacts
   - Homebrew and WinGet publishing

## Why These Are Pending

- The Mac-first core runtime is now working end to end, so the remaining work is mostly platform expansion and productization.
- Secure device identity is complete on macOS, but the other platforms still need the same non-exportable key treatment.
- Onboarding and governance are still design-heavy product layers rather than runtime plumbing.
- Release packaging is still required before public rollout.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.
