# Local Docs Site Preview

This page describes the local preview for the docs site while the public domain stays future-only.

MundusX is the product. `opengpu` is the current command family used by the local prototype, and the broader public OpenGPU ecosystem is treated as separate.

## Purpose

The docs site should be the future public-facing home for:

- install
- device identity
- releases

The current preview copy is Mac-first, so the local preview should keep that framing until the release channel expands.

## Local Preview

The dashboard app serves a local docs preview at:

- `http://127.0.0.1:<port>/docs`

It also mirrors the public-endpoint shape locally at:

- `http://127.0.0.1:<port>/public/docs`

Use that preview to review the copy before wiring the public domain.
The repository also owns a static export path now: `npm run build:docs-site` generates a GitHub Pages-ready site in `dist/public-docs-site`, and `.github/workflows/public-docs-site.yml` deploys that artifact from `uat` and `main`.

That static artifact now includes the same public-surface mirror paths as the localhost dashboard:

- `/public/docs`
- `/public/docs/install`
- `/public/install`
- `/public/install.json`
- `/public/install.sh`
- `/public/install.ps1`
- `/public/release`
- `/public/release.json`

This keeps the future public install endpoint and the GitHub-hosted release/distribution story reviewable in pull requests even before `mundusx.ai` is finally wired up.

## Review Rule

Any change to the docs site should keep the copy aligned with:

- `docs/install-page.md`
- `docs/device-identity-lifecycle.md`
- `docs/install-strategy.md`
