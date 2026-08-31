# Coding Harness v1 runner setup

The Harness runner is a separate user application. `opengpu-node-agent` remains inference-only
and never receives repository credentials, Git access, workspace authority, or validation tools.

## User setup

1. Install [Git](https://git-scm.com/) and the [GitHub CLI](https://cli.github.com/) on the machine
   that owns the user's workspace. For Java projects, also install a JDK and
   [Apache Maven](https://maven.apache.org/install.html), and ensure `java` and `mvn` are on `PATH`.
2. Authenticate that machine without copying a token into Chat-U:

   ```text
   gh auth login
   gh auth setup-git
   ```

3. Download the `mundusx-harness-runner` asset for the operating system from the dedicated
   `harness-runner-v*` MundusX release and verify its adjacent SHA-256 file.
4. In Chat-U, sign in with GitHub, open **Coding Harness**, and select **Create pairing code**.
5. Run the displayed command before its ten-minute expiry:

   ```text
   mundusx-harness-runner pair MX-<one-time-code>
   mundusx-harness-runner run
   ```

`pair` initializes the runner when needed. It creates a separate signing identity, discovers the
absolute Git and GitHub CLI paths, creates a private runner home under
`~/.mundusx/harness-runner`, and binds the runner to the authenticated Chat-U user. The control
plane obtains the user ID from the one-time pairing record; a runner cannot self-assert an owner.

The pairing secret is stored in PostgreSQL only as a SHA-256 digest, expires after ten minutes,
and can be consumed only once by one runner signing key. Re-registration uses that bound key and
owner and does not need another code.

## Repository flow

For a new project, Chat-U creates the repository under the signed-in user's GitHub identity. Private
is the default. MundusX does not own the repository, and the client cannot choose a different owner.
The GitHub App must have repository **Administration: read and write** permission for this operation;
installing it for all repositories makes the new repository immediately available to the Harness.

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

`status` reports pairing, GitHub CLI authentication, capability validation, repository source
patterns, and identity trust without printing credentials. Harness approval authorizes UAT
execution only; applying to production, merging, and deployment remain separately approved.
