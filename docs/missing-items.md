# Missing Items

This document tracks what is still missing before MundusX becomes a full product.

For the active working list, see [docs/master-checklist.md](master-checklist.md).
For broader next-step context, see [docs/pending-items.md](pending-items.md).

## CLI Phase

- [x] Rust CLI scaffold
- [x] Local config persistence
- [x] Local routing selection
- [x] One-click installer script
- [x] Release workflow for prebuilt binaries
- [x] Config setter commands
- [x] Contribution cap config
- [x] Device identity keypair reuse
- [x] macOS non-exportable device identity in encrypted-at-rest sign-only fallback
- [ ] macOS non-exportable device key storage in OS secure storage
- [x] macOS non-exportable identity enforcement policy
- [x] Windows DPAPI-protected device identity and operator-token storage
- [ ] Windows non-exportable key enforcement policy and implementation beyond DPAPI
- [ ] Linux protected device identity storage
- [x] Start-first onboarding flow
- [x] Friendly exit command
- [x] Reviewable official model catalog config
- [x] Signed release binaries published from tags
- [ ] Public install endpoint behind `mundusx.ai`
- [x] Packaging checks on macOS and Linux release artifacts
- [ ] Windows CLI release asset published from tags
- [ ] WinGet package publishing
- [ ] Homebrew tap or formula publishing
- [x] Final CLI help polish and error messaging

### CLI Follow-Up Issues

- Windows release asset: [mundusx/mundusx#73](https://github.com/mundusx/mundusx/issues/73)
- Windows release runner and signing decision: [mundusx/mundusx#109](https://github.com/mundusx/mundusx/issues/109)
- Homebrew channel publishing: [mundusx/mundusx#110](https://github.com/mundusx/mundusx/issues/110)
- WinGet package publishing: [mundusx/mundusx#111](https://github.com/mundusx/mundusx/issues/111)
- Linux protected identity storage: [mundusx/mundusx#113](https://github.com/mundusx/mundusx/issues/113)
- macOS identity policy: [docs/macos-identity-policy.md](macos-identity-policy.md)

## Control Plane Phase

- [x] Real control-plane API in Rust
- [x] Node registration endpoint
- [x] Heartbeat ingestion
- [x] Routing API
- [x] Device signature auth for node registration, heartbeat, claim, and completion requests
- [x] Operator auth for control-plane users
- [x] Job submission and tracking
- [x] Durable job event audit trail
- [x] Durable state store
- [x] Supabase migration runner and checked-in RLS rollout
- [x] Supabase restore path verification in the live project
- [x] Agent / worker contract defined in proto and CLI types

## Node Agent Phase

- [x] Local daemon in Rust
- [x] Heartbeat sender
- [x] Capability reporting
- [x] Pause/resume integration
- [x] Control-plane registration client
- [x] Safe throttling and policy enforcement

## Worker Phase

- [x] Local worker launch path scaffold
- [x] Contributor model cache manifest and switch commands
- [x] Real contributor model download backend
- [x] `M` worker adapter
- [x] Real execution payloads
- [x] Health checks for worker backends

## Product Phase

- [x] Dashboard for node and job visibility
- [x] Credits / accounting model
- [x] Onboarding flow
- [ ] Public docs site
- [ ] Branding and naming cleanup
- [ ] Federated governance / standards org model
