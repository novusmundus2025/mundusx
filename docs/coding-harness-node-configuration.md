# Coding Harness v1 runner configuration

The Harness runner is a separate application from the MundusX inference contributor.

- `opengpu-node-agent` contributes model inference only. It does not compile Harness modules,
  advertise repository authority, or store repository, workspace, Git, or validation settings.
- `mundusx-harness-runner` runs only on a user's trusted machine. It owns that user's repository
  access, temporary workspaces, Git operations, tools, tests, and validation limits.
- Model turns are requested from EHDA through `/v1/chat/completions`. Contributor nodes can perform
  inference on bounded prompts, but they never receive filesystem access, repository credentials,
  or authority to execute tools.

If no eligible user-owned runner is paired, the control plane returns
`HARNESS_RUNNER_UNAVAILABLE`. It never falls back to an inference contributor.

## Build the runner

The ordinary contributor release deliberately builds only `opengpu-node-agent`. Build the runner
explicitly on the user's trusted workstation:

```powershell
cargo build --release --manifest-path agents/node/Cargo.toml --features harness-runner --bin mundusx-harness-runner
```

Initialize its independent config and signing identity:

```powershell
.\target\release\mundusx-harness-runner.exe init
```

The runner uses `MUNDUSX_HARNESS_RUNNER_HOME` when set. Otherwise its files live under
`~/.mundusx/harness-runner`, separate from the contributor agent's `~/.opengpu` data.

## Configure the user's runner

Edit the generated `config.json`. All executable and repository paths must be absolute and must
refer to resources controlled by that user:

```json
{
  "version": 1,
  "runner_id": "runner-my-workstation",
  "device_id": "runner-device-my-workstation",
  "owner_user_id": "<authenticated-user-uuid>",
  "tenant_ids": ["tenant-personal"],
  "control_plane_url": "https://uat.mundusx.ai",
  "inference_model": "mundusx-agnostic",
  "parallel_slots": 1,
  "usable_memory_mb": 4096,
  "max_workspace_mb": 4096,
  "repositories": {
    "ehda-control-plane": "C:\\trusted-repositories\\control-plane"
  },
  "workspace_root": "C:\\mundusx-harness\\workspaces",
  "git_executable": "C:\\Program Files\\Git\\cmd\\git.exe",
  "validation_profiles": {
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
  },
  "sandbox_runtime": null,
  "sandbox_image_digest": null
}
```

Use a stable, unique runner ID. Runner identity, kind, and owning user are immutable; pair a new ID
if ownership changes. Repository source IDs are opaque control-plane identifiers mapped only to
repositories the user already controls. Start with one slot and increase it only after measuring
memory and I/O pressure.

For sandbox mode, set an absolute runtime path and a digest-pinned image:

```json
{
  "sandbox_runtime": "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe",
  "sandbox_image_digest": "registry.example/harness@sha256:<64-hex-digest>"
}
```

Without these sandbox fields, a correctly configured runner advertises hybrid mode only. Network
access remains disabled unless the user-owned validation profile explicitly enables it.

## Verify and run

```powershell
.\target\release\mundusx-harness-runner.exe status
.\target\release\mundusx-harness-runner.exe run --once
.\target\release\mundusx-harness-runner.exe run
```

The contributor agent's capability manifest must continue to report no Harness capability. Do not
put secrets in runner configuration. Harness execution approval remains UAT-only; applying,
merging, or deploying a returned patch requires separate control-plane approval.
