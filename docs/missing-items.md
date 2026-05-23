# Missing Items

This document tracks what is still missing before OpenGPU becomes a full product.

For the next implementation order, see [docs/pending-items.md](/Users/DBATALL/Documents/aigrid/docs/pending-items.md).

## CLI Phase

- [x] Rust CLI scaffold
- [x] Local config persistence
- [x] Local routing selection
- [x] One-click installer script
- [x] Release workflow for prebuilt binaries
- [x] Config setter commands
- [x] Contribution cap config
- [x] Device identity keypair reuse
- [x] Start-first onboarding flow
- [x] Friendly exit command
- [x] Reviewable official model catalog config
- [ ] Signed release binaries published from tags
- [ ] Public install endpoint behind `novusx.ai`
- [ ] Packaging checks on macOS and Linux release artifacts
- [ ] WinGet package publishing
- [ ] Homebrew tap or formula publishing
- [ ] Final CLI help polish and error messaging

## Control Plane Phase

- [x] Real control-plane API in Rust
- [x] Node registration endpoint
- [x] Heartbeat ingestion
- [x] Routing API
- [x] Device signature auth for node registration, heartbeat, claim, and completion requests
- [ ] Operator auth for control-plane users
- [x] Job submission and tracking
- [ ] Durable state store
- [ ] Supabase schema and RLS rollout
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
- [ ] Real execution payloads
- [ ] Health checks for worker backends

## Product Phase

- [ ] Dashboard for node and job visibility
- [ ] Credits / accounting model
- [ ] Onboarding flow
- [ ] Public docs site
- [ ] Branding and naming cleanup
