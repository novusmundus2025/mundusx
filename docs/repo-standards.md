# Repository Standards

This document is the living guide for folder structure, code ownership, and review expectations in the OpenGPU repo.

## Canonical Folder Structure

- `apps/cli/`
  - Rust CLI for contributor bootstrap, status, and local control
- `apps/control-plane/`
  - private Rust control-plane service, scheduler, and job queue
- `apps/dashboard/`
  - future operator web UI
- `agents/node/`
  - Rust node agent and local worker orchestration
- `workers/m-series/`
  - Mac / Apple Silicon execution path
- `workers/cuda/`
  - reserved for the later NVIDIA path
- `packages/shared/`
  - shared cross-package utility/types layer
- `packages/proto/`
  - protobuf and RPC contracts
- `supabase/`
  - SQL schema and migrations for the company-side database
- `docs/`
  - living architecture, policy, and product docs
- `tools/`
  - OS-specific helpers and build-time support code
- `scripts/`
  - install/bootstrap helper scripts

## Review Rules

Before merging code, verify:

1. The code lives in the right folder for its runtime.
2. The folder contains only the language and files it actually needs.
3. Public docs were updated when behavior changed.
4. Security-sensitive flows stay sign-only and do not expose raw secrets.
5. Tests pass.
6. New code has a clear owner and purpose.
7. Placeholder code does not leak into runtime trees that should already be real.

## Current Expectations

- Rust crates should not carry stray JS placeholders in their source trees.
- Dashboard can stay a minimal JS placeholder until the real UI is built.
- `agents/node` should be Rust-only.
- Security code must fail closed instead of falling back to exportable keys or silent stubs.
- Supabase schema changes should be versioned in `supabase/`.

## When To Update This Doc

Update this doc when:

- a package changes ownership
- a new runtime layer is added
- a placeholder becomes real code
- the review checklist changes
- a folder is renamed or split

