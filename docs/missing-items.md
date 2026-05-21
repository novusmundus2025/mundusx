# Missing Items

This document tracks what is still missing before OpenGPU becomes a full product.

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
- [ ] Signed release binaries published from tags
- [ ] Public install endpoint behind `novusx.ai`
- [ ] Packaging checks on macOS and Linux release artifacts
- [ ] Final CLI help polish and error messaging

## Control Plane Phase

- [ ] Real control-plane API in Rust
- [ ] Node registration endpoint
- [ ] Heartbeat ingestion
- [ ] Routing API
- [ ] Auth flow for users and devices
- [ ] Job submission and tracking
- [ ] Durable state store

## Node Agent Phase

- [ ] Local daemon in Rust
- [ ] Heartbeat sender
- [ ] Capability reporting
- [ ] Pause/resume integration
- [ ] Safe throttling and policy enforcement

## Worker Phase

- [ ] `M` worker adapter
- [ ] `CUDA` worker adapter
- [ ] Real execution payloads
- [ ] Health checks for worker backends

## Product Phase

- [ ] Dashboard for node and job visibility
- [ ] Credits / accounting model
- [ ] Onboarding flow
- [ ] Public docs site
- [ ] Branding and naming cleanup
