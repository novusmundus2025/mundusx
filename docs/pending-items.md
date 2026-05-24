# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

For the single working checklist, see [docs/master-checklist.md](/Users/DBATALL/Documents/aigrid/docs/master-checklist.md).

## Next Up

1. **Port secure device identity to all platforms**
   - macOS uses encrypted-at-rest device identity metadata and will use Keychain when available, but the long-term goal is still OS-backed secure storage everywhere
   - keep the private key non-exportable on Windows and Linux too
   - reuse the same sign-only identity model on the remaining platforms

2. **Define the federated governance model**
   - document the top-level standards / clearing-house org
   - document how operator companies join and certify
   - define settlement, revocation, and protocol versioning rules
   - keep the company control plane separate from the governance layer

3. **Public install and release rollout**
   - public install endpoint
   - Homebrew and WinGet publishing

## Why These Are Pending

- The Mac-first core runtime is now working end to end, so the remaining work is mostly platform expansion and productization.
- Secure device identity is sign-only and non-exportable on macOS with encrypted-at-rest storage and an optional Keychain secret, but the other platforms still need the same treatment.
- Onboarding and governance are still design-heavy product layers rather than runtime plumbing.
- The public install endpoint and package-manager publishing are still required before public rollout.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.
