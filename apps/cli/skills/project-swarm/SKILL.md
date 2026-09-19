---
name: project-swarm
description: Automatically coordinate substantial multi-part project changes or independent test lanes, and honor explicit requests for Project Swarm, parallel coding, or faster independent checks.
---

# Project Swarm

Use Project Swarm automatically when substantial project work can be divided safely, and for independent, read-only test or validation commands. Users do not need to know or name Project Swarm. An explicit request forces the coordinator to try; an explicit request for one worker or sequential work disables it.

## Coordinated coding

The connected-project runtime owns coding coordination. It asks a planner for a dependency graph, validates disjoint path ownership, creates isolated Git worktrees, runs up to two ready Hermes workers at once, integrates their commits in dependency order, and applies the integration only after its verification command passes.

Automatically attempt coordinated coding for complete APIs, frontends, backends, applications, websites, dashboards, full-stack work, end-to-end work, and changes spanning at least two of API, frontend, backend, database, authentication, tests, or documentation. Keep small focused edits on one worker.

Coding coordination requires a clean Git project. When the selected project root is a dirty repository without unresolved conflicts, create a local-only checkpoint commit before planning, report its short commit ID to the user, and use that exact checkpoint as the worker base. Never push this checkpoint automatically. Refuse to checkpoint a subfolder of a larger repository or a repository with unresolved conflicts; explain the exact condition and continue with the existing single Hermes worker. If Git is missing, the project is not a repository, or the plan has overlapping ownership, continue with the existing single Hermes worker. Do not ask the user to clean or commit the repository merely to enable parallelism.

For frontend work, split only along real source boundaries. Good worker ownership includes separate pages, component groups, API clients, assets, or test suites. Keep shared entry points, routing, dependency manifests, lockfiles, and final browser acceptance with the coordinator. A focused runtime repair centered on one entry point or one dependency boundary should remain with one worker; independent diagnosis or read-only checks may still run in parallel.

Before reporting why coding coordination fell back, inspect the exact coordinator error. Say whether the repository is dirty, Git is unavailable, fewer than two disjoint tasks were found, path ownership overlaps, a worker failed, or integration verification failed. Do not hide an actionable safety condition behind a generic partition message.

Workers must not publish, deploy, edit shared migrations, or update dependency lockfiles independently. The coordinator owns Git commits and integration. A worker failure, ownership violation, merge conflict, cancellation, or failed integrated verification leaves the project at the local pre-swarm checkpoint with the user's files preserved.

Frontend integration is incomplete until the production build has no unresolved-import or missing-export diagnostics and the changed flow renders in a browser without console exceptions. Exercise the requested interaction at the relevant viewport before accepting the integrated result.

## Parallel tests

Before starting tests, inspect the project test scripts and automatically use parallel lanes when commands can run independently. Do not wait for the user to request Project Swarm. Do not parallelize database migrations, tests sharing a mutable database, commands using the same fixed port, snapshot updates, dependency installation, builds writing to the same output directory, publishing, deployment, or other external writes. If independence is uncertain, use the project's normal sequential test command.

Run two workers by default:

```text
mundusx swarm test --workers 2 \
  --task "unit=npm run test:unit" \
  --task "api=npm run test:api" \
  --fallback "npm test"
```

On Windows, use the same command on one line. Each `--task` value is `NAME=COMMAND`. Use `--fail-fast` only when the user prefers quicker failure over collecting all results. Never exceed the number of independent lanes, and do not exceed eight workers.

A project can opt in persistently with `.mundusx/swarm.json`:

```json
{
  "version": 1,
  "test": {
    "enabled": true,
    "workers": 2,
    "fail_fast": false,
    "fallback": "npm test",
    "tasks": [
      { "name": "unit", "command": "npm run test:unit" },
      { "name": "api", "command": "npm run test:api" }
    ]
  }
}
```

The runner stores separate logs and temporary directories under `.hermes/swarm/`. Treat those as internal runtime files. Report failed lane names and their log paths. A failed lane is evidence that validation failed; do not hide it by editing tests or claiming the fallback passed unless it actually ran and passed.
