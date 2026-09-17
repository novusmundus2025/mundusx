---
name: project-swarm
description: Run independent project test or validation lanes concurrently when the user explicitly asks for parallel tests, faster independent checks, or Project Swarm, or when the project has enabled a MundusX swarm manifest.
---

# Project Swarm

Use Project Swarm for independent, read-only test or validation commands. Keep ordinary project implementation in the existing single-agent workflow.

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
