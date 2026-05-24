# Public Install Page

This page defines the public-facing first touch for `opengpu`.

## Purpose

The install page should do one thing well:

- explain that `opengpu` is installed with one command
- show the exact copy-paste install command
- set expectations that the installer downloads a signed Mac-first release binary
- point users to release notes and checksums

## Canonical Copy

The public page should present this command:

```bash
curl -fsSL https://novusx.ai/install | bash
```

## What The Page Is

- A simple public landing page behind `https://novusx.ai/install`
- The source of truth for the current install command
- The place users land before they ever see the CLI

## Local Preview

During development, the same copy is available from the dashboard server at:

- `http://127.0.0.1:<port>/install`

The local dashboard typically runs on `3001`, but you can override `PORT` during review.

This keeps the install page reviewable on localhost before the public endpoint is wired up.

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
5. The user runs `opengpu start` or `opengpu onboarding` next.

## Page Requirements

- Keep the page short and readable.
- Keep the command identical to the installer docs.
- Keep the wording Mac-first until the release channel expands.
- Link to release notes and checksums when available.

## Review Rule

Any change to the public install page must be reviewed against:

- `install.sh`
- `docs/install-strategy.md`
- `docs/who-runs-what.md`
- `README.md`
