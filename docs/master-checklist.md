# Master Checklist

This is the one checklist to track until the product is ready for broader release.

Use this document as the single working list:

- when an item is done, mark it done here first
- keep the phase docs for detail and architecture notes
- do not add new work here without also deciding where it belongs in the repo
- for a three-way comparison of implemented vs public launch vs federated network work, see [docs/gap-checklist.md](gap-checklist.md)

## Current Focus

1. **Public install endpoint behind `mundusx.ai`**
   - keep the public install page aligned with the localhost preview
   - keep the install command, checksum wording, and release links in sync
   - keep the public landing copy matching the current signed release flow

## Remaining Work

### Release and Install

- [ ] Public install endpoint behind `mundusx.ai`
- [x] Signed release binaries published from tags
- [x] Packaging checks on macOS and Linux release artifacts
- [ ] Windows CLI release asset published from tags ([mundusx/mundusx#73](https://github.com/mundusx/mundusx/issues/73))
- [ ] Homebrew tap or formula publishing
- [ ] WinGet package publishing
- [x] Final CLI help polish and error messaging

### Device Identity

- [ ] macOS non-exportable device key storage in OS secure storage
- [x] macOS non-exportable identity enforcement policy
- [x] Windows DPAPI-protected device identity and operator-token storage
- [ ] Windows non-exportable key enforcement policy and implementation beyond DPAPI
- [ ] Linux protected device identity storage
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
- Windows DPAPI-protected device identity and operator-token storage

## Active Follow-Up Issues

- Windows release asset: [mundusx/mundusx#73](https://github.com/mundusx/mundusx/issues/73)
- Windows release runner and signing decision: [mundusx/mundusx#109](https://github.com/mundusx/mundusx/issues/109)
- Homebrew channel publishing: [mundusx/mundusx#110](https://github.com/mundusx/mundusx/issues/110)
- WinGet package publishing: [mundusx/mundusx#111](https://github.com/mundusx/mundusx/issues/111)
- Linux protected identity storage: [mundusx/mundusx#113](https://github.com/mundusx/mundusx/issues/113)
- macOS identity policy: [docs/macos-identity-policy.md](macos-identity-policy.md)

## Tracking Rule

Work the items in order from top to bottom unless a later item is a dependency for an urgent fix.

When a section is finished:

1. Mark the item complete here.
2. Update the more detailed doc for that area.
3. Update the README links if the user-facing surface changed.
