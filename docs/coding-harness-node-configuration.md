# Coding Harness v1 node configuration

The node agent advertises Harness capability only after all trusted execution paths are configured.
Task payloads contain opaque source/profile IDs and can never supply repository URLs, executable
paths, validation commands, credentials, or sandbox images.

Add the following fields to the node's existing `config.json`. Use paths valid on that node:

```json
{
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

For sandbox mode, also configure an absolute runtime path and a digest-pinned image:

```json
{
  "harness_sandbox_runtime": "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe",
  "harness_sandbox_image_digest": "registry.example/harness@sha256:<64-hex-digest>"
}
```

Without the sandbox fields, a correctly configured trusted node advertises hybrid mode only. The
agent probes the pinned sandbox runtime before advertising sandbox support. Network access remains
disabled unless the operator-owned validation profile explicitly enables it.

After editing, restart the node agent and inspect its registration/heartbeat capability manifest.
Do not put secrets in these fields. Harness execution approval remains UAT-only; applying, merging,
or deploying a returned patch requires separate control-plane approval.
