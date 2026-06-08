# Priority Label Policy

This document defines the final priority labeling policy for MundusX backlog automation.

## Source Of Truth

- Use `.github/priority-label-policy.json` as the machine-readable source of truth.
- Use this document for the human-readable explanation.
- Use GitHub Project 3 as the active prioritization board for this repo.

## Priority Labels

Every open issue that is eligible for automation should carry exactly one of these labels:

- `priority:P0`
- `priority:P1`
- `priority:P2`
- `priority:P3`

The labels mean:

- `priority:P0` for blocking or release-critical work
- `priority:P1` for important current-milestone work
- `priority:P2` for near-term work that is useful but not blocking
- `priority:P3` for later or nice-to-have work

## Selection Rules

- Automation should select the highest-priority ready issue first.
- Automation must not select an issue that is explicitly blocked.
- Automation should require exactly one priority label before treating an issue as cleanly prioritized.
- Older issues may still use title prefixes such as `[P0]` through `[P3]`; automation may read those as a fallback until labels are backfilled.

## Area Guardrails

When triaging or creating issues, keep the board area aligned with one of:

- `Public`
- `Private`
- `Platform`
- `Workflow`
- `Docs`

If an issue spans more than one area, split it before automation tries to execute it.

## Issue Intake

- New issues should include a priority choice in the issue form.
- The matching GitHub label should be applied during triage.
- If the priority is unclear, the issue should stay blocked until the owner resolves it.

## Project Sync

- GitHub Project 3 is the board automation should sync against.
- The project sync workflow should support organization-owned projects under `mundusx`.
- Backlog items should map to the project status field without renaming the priority labels.
