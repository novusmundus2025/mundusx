# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

For the single working checklist, see [docs/master-checklist.md](master-checklist.md).

## Next Up

1. **Port secure device identity to all platforms**
   - macOS uses encrypted-at-rest device identity metadata and Keychain when available, with a remaining policy issue for when non-exportable storage is mandatory ([mundusx/mundusx#114](https://github.com/mundusx/mundusx/issues/114))
   - Windows now protects device identity and operator tokens with DPAPI; the remaining Windows work is the stricter non-exportable key policy and implementation beyond DPAPI
   - Linux still needs a protected sign-only identity path instead of the file-backed development prototype ([mundusx/mundusx#113](https://github.com/mundusx/mundusx/issues/113))

2. **Define the federated governance model**
   - document the top-level standards / clearing-house org
   - document how operator companies join and certify
   - define settlement, revocation, and protocol versioning rules
   - keep the company control plane separate from the governance layer

3. **Public install and release rollout**
   - public install endpoint
   - Windows CLI release asset from the signed tag workflow ([mundusx/mundusx#73](https://github.com/mundusx/mundusx/issues/73))
   - Homebrew publishing ([mundusx/mundusx#110](https://github.com/mundusx/mundusx/issues/110))
   - WinGet publishing ([mundusx/mundusx#111](https://github.com/mundusx/mundusx/issues/111))

## Why These Are Pending

- The Mac-first core runtime is now working end to end, so the remaining work is mostly platform expansion and productization.
- Secure device identity now avoids plaintext private-key persistence on macOS and Windows. Linux still needs protected storage, and macOS/Windows still need clear enterprise enforcement policy for when fallback storage is allowed.
- Onboarding and governance are still design-heavy product layers rather than runtime plumbing.
- The public install endpoint and package-manager publishing are still required before public rollout.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.
