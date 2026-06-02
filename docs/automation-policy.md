# MundusX Automation Policy

This document defines how repository maintenance automation should decide whether a run is successful, blocked, or eligible to auto-continue.

## Repository Type

`mundusx/mundusx` is a CLI and distribution repo.

That means the primary outputs are:

- CLI code
- release artifacts
- installer behavior
- localhost/public-docs preview surfaces

It is **not** primarily a long-running web service repo.

## Success Criteria For This Repo

An automation run for this repo is considered successful when all of the following are true:

1. the selected issue is clear and repo-local
2. the change merges cleanly into `uat`
3. the relevant repo validations pass
4. the relevant artifact or smoke verification passes
5. the PR and branch cleanup complete, or any cleanup miss is limited to a clearly transient GitHub failure

Examples of acceptable post-merge verification for this repo:

- `bash tests/license-surface.sh`
- `bash scripts/localhost-smoke.sh`
- `bash scripts/verify-release-packaging.sh`

The automation should choose the smallest verification set that proves the selected issue.

## What Is Not A Blocker Here

For this repo alone, the following must **not** force a blocker outcome by themselves:

- missing `uat` web service URL
- missing deployment health endpoint
- missing live web deployment fingerprint

Those checks apply to service repos, not to this CLI/distribution repo.

## Hard Blockers

Automation should stop and send a blocker outcome when one of these remains unresolved:

- failing validations
- ambiguous issue selection
- missing credentials required for the selected issue
- merge conflicts that require new code changes
- missing configuration required to execute the selected issue safely

## Transient Blockers

Automation should retry before declaring a blocker when the failure is likely temporary:

- GitHub API or auth flake
- git transport failure
- network or DNS failure
- CI flake
- rate limit

## Source Of Truth

The machine-readable source of truth for this policy lives in:

- `.github/automation-policy.json`

Use this document for human explanation and the JSON file for automation-specific interpretation.

## Priority Labeling Policy

Backlog priority for this repo is managed from GitHub Project 3.

Automation should treat these labels as the allowed repo-level priority set:

- `priority:P0`
- `priority:P1`
- `priority:P2`

Default rule:

- if an issue does not yet have one of those labels, treat it as `priority:P1` unless GitHub Project 3 explicitly says otherwise

Selection rule:

- choose the highest-priority repo-local issue from GitHub Project 3
- if several issues have the same priority, choose the smallest clearly shippable change
