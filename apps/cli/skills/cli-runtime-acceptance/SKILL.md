---
name: cli-runtime-acceptance
description: Build, repair, and verify command-line programs, especially interactive CLIs that read prompts, menus, secrets, or repeated input from stdin.
---

# CLI Runtime Acceptance

Use this skill whenever a command-line or console program is created or changed, or when a program run hangs, prints usage unexpectedly, waits for input, or exits with a nonzero status.

Keep business logic separate from interactive input and output so tests can call it directly. Validate encryption, parsing, storage, and other behavior with deterministic project-owned automated tests. Do not rely on a manually operated terminal session as the regression test.

Before running the program, inspect its documented arguments and input flow. A usage message with a nonzero exit status means the invocation is wrong; correct the invocation rather than repeating it.

Never launch an interactive CLI as an unattended foreground verification while it can wait for stdin. For a smoke test, supply every expected response through redirected input, a here-document, or the program's supported noninteractive flags. Set an explicit timeout of at most 30 seconds. Do not include real secrets in the test input.

After the final source change:

1. Run the project-owned automated test suite successfully.
2. Run a bounded noninteractive smoke test that covers the requested CLI flow, including both directions of a reversible operation when applicable.
3. Confirm the process exits and its output demonstrates the requested behavior without exposing secret input.
4. If a check fails, use the concrete exit status and output to repair the source, then repeat the complete test and smoke sequence.
5. Return the final result immediately after the final checks pass.
