# Naming Strategy

This repo is building the MundusX product and its local prototype surfaces.

## Current Rule

- `MundusX` is the product and platform name we should use on user-facing surfaces.
- `opengpu` is the current CLI command and compatibility binary name that already exists in the repo.
- `OpenGPU` is treated as legacy / compatibility wording inside older code paths, sample data, or references to the broader public ecosystem.

## Why This Matters

There is already a separate public OpenGPU Network presence on the internet.

To avoid confusion:

- the local UI, docs, and public-facing copy should say `MundusX`
- the current command name can remain `opengpu` until we intentionally decide to rename the binary family
- if we ever rename the binary to `mundusx`, do it as a deliberate compatibility pass, not as an incidental string swap

## Practical Guidance

- Use `MundusX` in headings, brand labels, page titles, and marketing copy.
- Keep `opengpu` in shell commands, tests, scripts, and code paths that already depend on the current binary name.
- If a doc mentions both, prefer wording like:
  - "MundusX is the product; `opengpu` is the current CLI command name."

## Current Public Surfaces To Watch

- install page
- docs site
- dashboard
- contributor portal
- control plane root page
- local release preview

## Migration Note

If we later rename the binary family:

1. add a `mundusx` alias first
2. keep `opengpu` as compatibility for a transition period
3. update docs and installers after the alias is stable
