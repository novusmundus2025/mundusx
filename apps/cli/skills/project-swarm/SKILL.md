---
name: project-swarm
description: Coordinate independent coding work or test lanes when the user explicitly asks for Project Swarm, parallel coding, or faster independent checks, or when the project has enabled a MundusX swarm test manifest.
---

# Project Swarm

Use Project Swarm for explicitly requested parallel coding and for independent, read-only test or validation commands.

## Coordinated coding

The connected-project runtime owns coding coordination. It asks a planner for a dependency graph, validates disjoint path ownership, creates isolated Git worktrees, runs up to two ready Hermes workers at once, integrates their commits in dependency order, and applies the integration only after its verification command passes.

Coding coordination requires a clean Git project. If the repository is missing, dirty, unsafe to partition, or the plan has overlapping ownership, continue with the existing single Hermes worker. Do not ask the user to clean or commit the repository merely to enable parallelism.

Workers must not publish, deploy, edit shared migrations, or update dependency lockfiles independently. The coordinator owns Git commits and integration. A worker failure, ownership violation, merge conflict, cancellation, or failed integrated verification leaves the original project revision unchanged.

## Parallel tests

Before starting parallel lanes, inspect the project test scripts and identify commands that can run independently. Do not parallelize database migrations, tests sharing a mutable database, commands using the same fixed port, snapshot updates, dependency installation, builds writing to the same output directory, publishing, deployment, or other external writes. If independence is uncertain, use the project's normal sequential test command.

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
