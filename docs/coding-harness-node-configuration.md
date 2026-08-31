# Coding Harness v1 runner setup

The Harness runner is a separate user application. `opengpu-node-agent` remains inference-only
and never receives repository credentials, Git access, workspace authority, or validation tools.

## User setup

1. Install [Git](https://git-scm.com/) on the machine that owns the user's workspace. GitHub CLI is
   optional and is needed only when the user later chooses to publish a project to GitHub. For Java projects, also install a JDK and
   [Apache Maven](https://maven.apache.org/install.html), and ensure `java` and `mvn` are on `PATH`.
2. Optional: authenticate GitHub on that machine when publication is wanted, without copying a token into Chat-U:

   ```text
   gh auth login
   gh auth setup-git
   ```

3. Download the `mundusx-harness-runner` asset for the operating system from the dedicated
   `harness-runner-v*` MundusX release and verify its adjacent SHA-256 file.
4. In Chat-U, open **Projects**, expand local runner setup, and select **Create pairing code**.
5. Run the displayed command before its ten-minute expiry:

   ```text
   mundusx-harness-runner pair MX-<one-time-code>
   mundusx-harness-runner run
   ```

`pair` initializes the runner when needed. It creates a separate signing identity, discovers the
absolute Git path (and GitHub CLI when installed), creates a private runner home under
`~/.mundusx/harness-runner`, and binds the runner to the authenticated Chat-U user. The control
plane obtains the user ID from the one-time pairing record; a runner cannot self-assert an owner.

The pairing secret is stored in PostgreSQL only as a SHA-256 digest, expires after ten minutes,
and can be consumed only once by one runner signing key. Re-registration uses that bound key and
owner and does not need another code.

## Local-first project flow

Projects start on the user's device under `documents/mundusx/projects/<lowercase-slug>`. Chat-U
sends only an owner-bound opaque project ID, template, objective, and bounded capabilities. The
paired runner creates the direct child folder, scaffolds the selected template, and initializes a
local Git history for rollback. Contributor nodes never receive the project files or credentials.

On each signed registration heartbeat, the runner reports at most 100 validated project slugs from
that direct-child folder. A directory is included only when its lowercase slug matches the name in
`.mundusx/project.json`; symlinks, nested folders, malformed metadata, and unmanaged directories are
ignored. The control plane binds this inventory to the runner's authenticated owner so Chat-U can
offer that user a multi-project picker without exposing local paths or another user's projects.

Each attempt runs in a unique no-hardlink temporary workspace under the private runner home. After
the bounded validation succeeds, the runner verifies that the source project has not changed,
commits the validated workspace, fast-forwards the local project, and deletes the temporary
workspace. Failed, cancelled, stale, or out-of-bound attempts do not alter the project folder.

GitHub publication is a separate optional action. If requested later, the repository is created
under the user's authenticated GitHub identity; MundusX does not own it.

## Existing GitHub repository flow

For existing projects, Chat-U lists only repositories visible to both the signed-in user and the
installed MundusX GitHub App. Chat-U verifies current GitHub permissions, assigns bounded project
paths, and pins the current default-branch commit. MundusX platform repositories have no special
status and appear only when that user deliberately granted the App access to them.

The paired runner receives an opaque source ID such as `github:12345:owner/repository`. It uses the
local GitHub CLI session to create a bare, commit-addressed cache under the runner home, then creates
a unique no-hardlink workspace from that cache. It disables hooks, global/system Git config,
terminal prompts, submodule execution, external diff, and text conversion for workspace actions.
No GitHub token is sent to model contributors or included in a prompt.

By default, the runner exposes a bounded `repository-default` validation (`git diff --check HEAD`)
and, when Maven is discovered during initial configuration, `java-maven-test` (`mvn --batch-mode
test`). Maven validation uses trusted hybrid execution because dependency resolution can require
network access. Sandbox execution is advertised only when the operator configures an absolute
container runtime and digest-pinned image in the generated `config.json`.

## Diagnostics

```text
mundusx-harness-runner status
mundusx-harness-runner run --once
```

`status` reports pairing, optional GitHub CLI authentication, capability validation, repository source
patterns, and identity trust without printing credentials. Harness approval authorizes UAT
execution only; applying to production, merging, and deployment remain separately approved.
