# Coding Harness v1 runner setup

The Harness runner is a separate user application. `opengpu-node-agent` remains inference-only
and never receives repository credentials, Git access, workspace authority, or validation tools.

## User setup

1. Install [Git](https://git-scm.com/) on the machine that owns the user's workspace. GitHub CLI is
   optional and is needed only when the user later chooses to publish a project to GitHub. For Java projects, also install a JDK and
   [Apache Maven](https://maven.apache.org/install.html), and ensure `java` and `mvn` are on `PATH`.
2. Optional: authenticate GitHub on that machine when publication is wanted, without copying a token into Chat:

   ```text
   gh auth login
   gh auth setup-git
   ```

3. In Chat, create or select a Project and ask for a local action such as creating files or running tests.
4. Select **Connect this computer**. On Windows, run the downloaded lightweight setup executable.
   On macOS or Linux, download `install-harness-runner.sh`, verify its adjacent SHA-256 file, and run it.
5. Approve the named computer in the browser page that opens. Chat resumes the original request
   automatically after the signed runner heartbeat arrives.

For headless diagnostics, the same browser-approved flow is available directly:

   ```text
   mundusx-harness-runner bootstrap --chat-url https://chat.mundusx.ai
   ```

`bootstrap` initializes the runner when needed. It creates a separate signing identity, discovers the
absolute Git path (and GitHub CLI when installed), creates a private runner home under
`~/.mundusx/harness-runner`, and binds the runner to the authenticated Chat user. It also configures
per-user startup (Windows Run, a macOS LaunchAgent, or a Linux systemd user service) and starts the
claim loop. The control plane obtains the user ID from the approved bootstrap record; a runner cannot self-assert an owner.

The bootstrap secret and browser approval token are stored in PostgreSQL only as independent SHA-256
digests. Both expire after ten minutes. Approval binds the session to the runner public key, and the
registration secret can be consumed only once by that runner. Re-registration uses the bound key and
owner and does not need another approval. The legacy `pair` command remains for recovery only.

## Local-first project flow

Projects start on the user's device under `documents/mundusx/projects/<lowercase-slug>`. Chat
sends only an owner-bound opaque project ID, template, objective, and bounded capabilities. The
paired runner creates the direct child folder, scaffolds the selected template, and initializes a
local Git history for rollback. Contributor nodes never receive the project files or credentials.

On each signed registration heartbeat, the runner reports at most 100 validated project slugs from
that direct-child folder. A directory is included only when its lowercase slug matches the name in
`.mundusx/project.json`; symlinks, nested folders, malformed metadata, and unmanaged directories are
ignored. The control plane binds this inventory to the runner's authenticated owner so Chat can
offer that user a multi-project picker without exposing local paths or another user's projects.

Each attempt runs in a unique no-hardlink temporary workspace under the private runner home. After
the bounded validation succeeds, the runner verifies that the source project has not changed,
commits the validated workspace, fast-forwards the local project, and deletes the temporary
workspace. Failed, cancelled, stale, or out-of-bound attempts do not alter the project folder.

GitHub publication is a separate optional action. If requested later, the repository is created
under the user's authenticated GitHub identity; MundusX does not own it.

## Existing GitHub repository flow

For existing projects, Chat lists only repositories visible to both the signed-in user and the
installed MundusX GitHub App. Chat verifies current GitHub permissions, assigns bounded project
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

### Public runner releases

User-facing binaries are published in `mundusx/releases`; the application source remains private.
Installers use the stable `https://downloads.mundusx.ai/prod/latest/<asset>` channel, which redirects
only allowlisted asset names to the latest public release. GitHub's URL is an internal storage detail.
Configure the private source repository Actions secret `MUNDUSX_PUBLIC_RELEASES_TOKEN` with a
fine-grained token granting Contents read/write access only to `mundusx/releases`. The release
workflow retains an internal prerelease in the source repository as an audit copy.

```text
mundusx-harness-runner status
mundusx-harness-runner run --once
```

`status` reports pairing, optional GitHub CLI authentication, capability validation, repository source
patterns, and identity trust without printing credentials. Harness approval authorizes UAT
execution only; applying to production, merging, and deployment remain separately approved.
