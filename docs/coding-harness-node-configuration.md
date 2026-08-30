# Coding Harness v1 local runner configuration

An inference contributor is not a Harness runner. Contributor registration advertises model
inference only and never exposes repositories, workspaces, credentials, Git, validation commands,
or Harness slots. A user may explicitly enable a separate local runner in the same installation;
its identity, ownership, scopes, heartbeat, and concurrency are registered independently.

If no eligible runner is paired, the control plane returns `HARNESS_RUNNER_UNAVAILABLE`. It never
falls back to an inference contributor.

Add the following fields to the node's existing `config.json`. Use paths valid on that node:

```json
{
  "harness_runner_id": "runner-my-workstation",
  "harness_runner_owner_user_id": "<authenticated-user-uuid>",
  "harness_runner_tenant_ids": ["tenant-personal"],
  "harness_runner_slots": 1,
  "harness_repositories": {
    "ehda-control-plane": "C:\\trusted-repositories\\control-plane"
  },
  "harness_workspace_root": "C:\\mundusx-harness\\workspaces",
  "harness_git_executable": "C:\\Program Files\\Git\\cmd\\git.exe",
  "harness_validation_profiles": {
    "rust-default": {
      "executable": "C:\\Users\\operator\\.cargo\\bin\\cargo.exe",
      "arguments": ["test", "--workspace"],
      "working_directory": "",
      "environment": {},
      "network_allowed": false,
      "timeout_ms": 900000,
      "max_output_bytes": 1048576,
      "max_memory_mb": 8192,
      "max_cpu_time_ms": 600000,
      "max_processes": 64
    }
  }
}
```

Use a stable, unique runner ID. The runner's device key, kind, and owning user are immutable; pair
a new ID when ownership changes. Repository source IDs are opaque control-plane identifiers and map
only to clones the user already controls locally. Start with one runner slot even when the same
machine also contributes inference, then increase it only after measuring memory and I/O pressure.

For sandbox mode, also configure an absolute runtime path and a digest-pinned image:

```json
{
  "harness_sandbox_runtime": "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe",
  "harness_sandbox_image_digest": "registry.example/harness@sha256:<64-hex-digest>"
}
```

Without the sandbox fields, a correctly configured trusted runner registers hybrid mode only. The
agent probes the pinned sandbox runtime before registering sandbox support. Network access remains
disabled unless the user-owned validation profile explicitly enables it.

After editing, restart the agent. Logs should show `harnessRunner: ready` separately from `jobPoll`.
The contributor capability manifest must continue to show no Harness capability. Do not put secrets
in these fields. Harness execution approval remains UAT-only; applying, merging, or deploying a
returned patch requires separate control-plane approval.
