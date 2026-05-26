# Gap Checklist

This document groups the remaining work into three buckets:

- already in the repo
- missing for public launch
- missing for a federated multi-company network

Use this alongside [docs/master-checklist.md](/Users/DBATALL/Documents/aigrid/docs/master-checklist.md) as the comparison sheet for what exists now versus what still needs to be built.

## Already In The Repo

These pieces already exist in the current local prototype and should be treated as implemented:

- CLI prototype
- node agent
- Mac-first worker path
- control plane API
- Supabase schema and migrations
- dashboard preview
- contributor portal preview
- requestor API adapter
- credits ledger
- onboarding flow
- local install page preview
- local docs preview
- local release preview helper
- localhost smoke test
- release packaging checks
- signed release manifest flow
- worker health propagation
- real execution payloads
- durable job event audit trail
- device identity lifecycle notes

## Missing For Public Launch

These are the remaining steps before the product feels public, polished, and distributable:

- public install endpoint
- public docs site
- public release hosting and distribution
- package manager distribution
  - Homebrew tap or formula
  - WinGet package
- contributor earnings portal polish
  - searchable, paginated job history
  - payout / withdrawal history
- requestor API completion
  - `GET /v1/models`
  - streaming responses
  - retry / timeout / idempotency rules
- contributor onboarding polish
- branding and naming cleanup
- production observability and release monitoring

## Missing For A Federated Multi-Company Network

These are the larger ecosystem pieces needed if multiple operator companies and a top-level broker/gateway are part of the product:

- org-managed gateway / broker
- company provider discovery
- cross-company health checking
- automatic failover between providers
- provider certification and trust policy
- operator-company settlement and revocation rules
- federated routing policy
- multi-company requestor routing modes
  - auto
  - pinned provider
  - direct provider
- standards org / governance layer
- protocol versioning rules
- audit and dispute handling across operators
- contributor roaming across companies
- shared settlement / clearing house model

## Naming Note

The current local prototype still uses the `opengpu` CLI and route names because that is what the repo already implements and tests.

If the product is rebranded later, `novusx` can become the user-facing brand or command family, but that should be an intentional rename pass rather than an incidental change.

## How To Use This Doc

- If something is already built, keep it in the first section.
- If it is needed for launch, place it in the second section.
- If it is only needed for the broader federated network, place it in the third section.
- When an item is completed, move it out of the missing sections and into the implemented section with a brief note in the related architecture doc.
