# Contributor Onboarding

The CLI now includes an explicit onboarding flow for contributors.

## What It Covers

- device identity and public fingerprint
- hostname as signed metadata
- selected backend and active model
- contribution cap
- policy state
- credits ledger link
- dashboard link

## Commands

- `opengpu onboarding` shows the onboarding checklist
- `opengpu onboarding --complete` marks onboarding complete
- `opengpu onboarding --reset` reopens the checklist

## Startup Behavior

If onboarding has not been completed yet, `opengpu start` and `opengpu connect` print the checklist and a hint to complete it later.

## Notes

- The onboarding state is local to the contributor machine.
- It does not change the device identity or the control plane record.
- This is a review checklist, not an extra login step.
