# Master Checklist

This is the one checklist to track until the product is ready for broader release.

Use this document as the single working list:

- when an item is done, mark it done here first
- keep the phase docs for detail and architecture notes
- do not add new work here without also deciding where it belongs in the repo
- for a three-way comparison of implemented vs public launch vs federated network work, see [docs/gap-checklist.md](/Users/DBATALL/Documents/mundusx/docs/gap-checklist.md)

## Current Focus

1. **Public install endpoint behind `novusx.ai`**
   - keep the public install page aligned with the localhost preview
   - keep the install command, checksum wording, and release links in sync
   - keep the public landing copy matching the current signed release flow

## Remaining Work

### Release and Install

- [ ] Public install endpoint behind `novusx.ai`
- [x] Signed release binaries published from tags
- [x] Packaging checks on macOS and Linux release artifacts
- [ ] Homebrew tap or formula publishing
- [ ] WinGet package publishing
- [x] Final CLI help polish and error messaging

### Device Identity

- [ ] macOS non-exportable device key storage in OS secure storage
- [ ] Windows/Linux non-exportable device key storage in OS secure storage
- [x] Verify the live Supabase restore path in the project again after any schema or secret changes

### Control Plane

- [x] Reconfirm the control plane boot source after release-side changes

### Worker

- [x] Real execution payloads
- [x] Health checks for worker backends

### Product Surface

- [ ] Public docs site
- [ ] Branding and naming cleanup

### Governance

- [ ] Federated governance / standards org model
- [ ] Formalize operator-company certification and settlement rules

## Done Already

The following major pieces are already in place and should stay marked complete:

- CLI prototype
- Node agent prototype
- Mac-first worker path
- Control plane API
- Supabase schema and migrations
- Dashboard preview
- Credits ledger
- Onboarding flow
- Release workflow
- Local install page preview
- Local release preview helper and CLI help polish
- Packaging checks on macOS and Linux release artifacts
- Signed release binaries published from tags

## Tracking Rule

Work the items in order from top to bottom unless a later item is a dependency for an urgent fix.

When a section is finished:

1. Mark the item complete here.
2. Update the more detailed doc for that area.
3. Update the README links if the user-facing surface changed.
