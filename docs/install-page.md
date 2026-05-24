# Local Install Page Preview

This page defines the localhost-first touch for `opengpu` while we keep the public domain for later.

## Purpose

The install page should do one thing well:

- explain that `opengpu` is installed with one command
- show the exact copy-paste install command
- set expectations that the installer downloads a signed Mac-first release binary from localhost during development
- point users to release notes and checksums

## Canonical Copy

The local page should present this command:

```bash
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download bash install.sh
```

## What The Page Is

- A simple local landing page behind `http://127.0.0.1:<port>/install`
- The source of truth for the current install command
- The place users land before they ever see the CLI

## Local Preview

During development, the same copy is available from the dashboard server at:

- `http://127.0.0.1:<port>/install`

The local dashboard typically runs on `3001`, but you can override `PORT` during review.
For the matching localhost release source, run `scripts/local-release-preview.sh up` to build and serve the repo-managed preview on `http://127.0.0.1:8788/releases/latest/download/`.

This keeps the install page reviewable on localhost before the public endpoint is wired up.
The same manifest-driven shell is also mirrored at `http://127.0.0.1:<port>/public/install` so the future public endpoint shape stays local for now.
The broader docs copy is previewable locally from the dashboard at `http://127.0.0.1:<port>/docs`.
The install page itself is now a shell that loads its command and release metadata from the machine-readable manifest at `http://127.0.0.1:<port>/install.json` so the HTML, installer, and release source stay in sync.
The matching localhost release preview is also manifest-driven and loads its visible release details from `http://127.0.0.1:8788/releases/latest/download/release-manifest.json`.
For a one-shot local verification pass, run `scripts/localhost-smoke.sh`.

For end-to-end localhost testing, point `install.sh` at a local release source with:

```bash
RELEASE_BASE_URL=http://127.0.0.1:8788/releases/latest/download
```

## What The Page Is Not

- Not the installer itself
- Not the release artifact store
- Not a dashboard
- Not a control plane

## Expected Flow

1. User opens the install page.
2. The page shows the one-line install command.
3. The command downloads the matching signed release binary.
4. The installer verifies the checksum when available.
5. The user runs `opengpu onboarding`.
6. The user runs `opengpu cap` to choose the contribution budget.
7. The user runs `opengpu start` to bring the machine online.

## Page Requirements

- Keep the page short and readable.
- Keep the command identical to the installer docs.
- Keep the wording Mac-first until the release channel expands.
- Link to release notes and checksums when available.
- Make the cap step obvious so a fresh contributor knows what to do before `start`.

## Review Rule

Any change to the local install page must be reviewed against:

- `install.sh`
- `docs/install-strategy.md`
- `docs/who-runs-what.md`
- `README.md`
