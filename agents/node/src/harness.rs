//! Coding Harness v1 repository workspace isolation.
//!
//! This module deliberately does not expose a shell. It prepares an immutable
//! Git revision in a unique workspace and confines later typed operations to
//! explicitly allowed path prefixes. Command/test execution is owned by the
//! bounded validation runner rather than this workspace manager.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use uuid::Uuid;

pub const HARNESS_CONTRACT_VERSION: &str = "1.0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLimits {
    pub max_disk_mb: u32,
    pub max_memory_mb: u32,
    pub max_cpu_time_ms: u64,
    pub max_wall_time_ms: u64,
}

impl Default for WorkspaceLimits {
    fn default() -> Self {
        Self {
            max_disk_mb: 4_096,
            max_memory_mb: 8_192,
            max_cpu_time_ms: 600_000,
            max_wall_time_ms: 900_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRequest {
    pub attempt_id: String,
    pub source_repository: PathBuf,
    pub base_revision: String,
    pub allowed_path_prefixes: Vec<PathBuf>,
    pub limits: WorkspaceLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessError {
    pub code: &'static str,
    pub message: String,
}

impl HarnessError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HarnessError {}

#[derive(Clone, Debug)]
pub struct WorkspaceManager {
    root: PathBuf,
    git_executable: PathBuf,
}

#[derive(Clone, Debug)]
pub struct PreparedWorkspace {
    attempt_id: String,
    path: PathBuf,
    base_revision: String,
    allowed_path_prefixes: Vec<PathBuf>,
    limits: WorkspaceLimits,
}

impl WorkspaceManager {
    pub fn new(root: PathBuf, git_executable: PathBuf) -> Result<Self, HarnessError> {
        if !git_executable.is_absolute() {
            return Err(HarnessError::new(
                "HARNESS_POLICY_DENIED",
                "the Git executable must use an absolute trusted path",
            ));
        }
        fs::create_dir_all(&root).map_err(|error| {
            HarnessError::new(
                "HARNESS_WORKSPACE_PREPARE_FAILED",
                format!("failed to create workspace root: {error}"),
            )
        })?;
        let root = fs::canonicalize(&root).map_err(|error| {
            HarnessError::new(
                "HARNESS_WORKSPACE_PREPARE_FAILED",
                format!("failed to resolve workspace root: {error}"),
            )
        })?;
        let disabled_hooks = root.join(".disabled-hooks");
        fs::create_dir_all(&disabled_hooks).map_err(|error| {
            HarnessError::new(
                "HARNESS_WORKSPACE_PREPARE_FAILED",
                format!("failed to create the disabled-hooks directory: {error}"),
            )
        })?;
        Ok(Self {
            root,
            git_executable,
        })
    }

    pub fn prepare(&self, request: WorkspaceRequest) -> Result<PreparedWorkspace, HarnessError> {
        validate_attempt_id(&request.attempt_id)?;
        validate_revision(&request.base_revision)?;
        let allowed_path_prefixes = normalize_allowed_prefixes(&request.allowed_path_prefixes)?;
        validate_limits(&request.limits)?;

        let source_repository = fs::canonicalize(&request.source_repository).map_err(|error| {
            HarnessError::new(
                "HARNESS_BASE_REVISION_INVALID",
                format!("repository source could not be resolved: {error}"),
            )
        })?;
        if !source_repository.is_dir() || source_repository.starts_with(&self.root) {
            return Err(HarnessError::new(
                "HARNESS_BASE_REVISION_INVALID",
                "repository source must be a directory outside the workspace root",
            ));
        }

        let name = format!("{}-{}", request.attempt_id, Uuid::new_v4().simple());
        let staging_path = self.root.join(format!(".{name}.partial"));
        let final_path = self.root.join(name);
        ensure_direct_child(&self.root, &staging_path)?;
        ensure_direct_child(&self.root, &final_path)?;

        let prepare_result = (|| {
            let source_text = git_path_text(&source_repository)?;
            let staging_text = git_path_text(&staging_path)?;
            let clone = self.run_git(
                None,
                &[
                    "clone",
                    "--no-checkout",
                    "--no-hardlinks",
                    "--local",
                    source_text.as_str(),
                    staging_text.as_str(),
                ],
            )?;
            require_git_success(&clone, "clone repository into isolated workspace")?;

            let checkout = self.run_git(
                Some(&staging_path),
                &["checkout", "--detach", request.base_revision.as_str()],
            )?;
            require_git_success(&checkout, "check out immutable base revision")?;

            let head = self.run_git(Some(&staging_path), &["rev-parse", "HEAD"])?;
            require_git_success(&head, "verify immutable base revision")?;
            let actual_revision = String::from_utf8_lossy(&head.stdout)
                .trim()
                .to_ascii_lowercase();
            if actual_revision != request.base_revision.to_ascii_lowercase() {
                return Err(HarnessError::new(
                    "HARNESS_BASE_REVISION_INVALID",
                    "checked-out revision did not match the requested immutable revision",
                ));
            }

            fs::rename(&staging_path, &final_path).map_err(|error| {
                HarnessError::new(
                    "HARNESS_WORKSPACE_PREPARE_FAILED",
                    format!("failed to publish prepared workspace: {error}"),
                )
            })?;
            Ok(())
        })();

        if let Err(error) = prepare_result {
            let _ = remove_workspace_directory(&self.root, &staging_path);
            return Err(error);
        }

        Ok(PreparedWorkspace {
            attempt_id: request.attempt_id,
            path: final_path,
            base_revision: request.base_revision.to_ascii_lowercase(),
            allowed_path_prefixes,
            limits: request.limits,
        })
    }

    pub fn cleanup(&self, workspace: &PreparedWorkspace) -> Result<(), HarnessError> {
        remove_workspace_directory(&self.root, &workspace.path)
    }

    pub fn repository_status(
        &self,
        workspace: &PreparedWorkspace,
        max_output_bytes: usize,
    ) -> Result<String, HarnessError> {
        self.require_owned_workspace(workspace)?;
        let output = self.run_git(
            Some(workspace.path()),
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )?;
        require_git_success(&output, "read repository status")?;
        bounded_git_stdout(&output, max_output_bytes)
    }

    pub fn repository_diff(
        &self,
        workspace: &PreparedWorkspace,
        max_output_bytes: usize,
    ) -> Result<String, HarnessError> {
        self.require_owned_workspace(workspace)?;
        let output = self.run_git(
            Some(workspace.path()),
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--src-prefix=a/",
                "--dst-prefix=b/",
            ],
        )?;
        require_git_success(&output, "read repository diff")?;
        bounded_git_stdout(&output, max_output_bytes)
    }

    pub fn repository_complete_diff(
        &self,
        workspace: &PreparedWorkspace,
        max_output_bytes: usize,
    ) -> Result<String, HarnessError> {
        self.require_owned_workspace(workspace)?;
        require_git_success(
            &self.run_git(Some(workspace.path()), &["add", "--all"])?,
            "stage project changes for evidence",
        )?;
        let output = self.run_git(
            Some(workspace.path()),
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--src-prefix=a/",
                "--dst-prefix=b/",
            ],
        )?;
        require_git_success(&output, "read complete repository diff")?;
        bounded_git_stdout(&output, max_output_bytes)
    }

    pub fn publish_validated_changes(
        &self,
        workspace: &PreparedWorkspace,
        source_repository: &Path,
        task_id: &str,
    ) -> Result<Option<String>, HarnessError> {
        self.require_owned_workspace(workspace)?;
        let status = self.repository_status(workspace, 1_048_576)?;
        if status.trim().is_empty() {
            return Ok(None);
        }
        for line in status.lines() {
            let value = line.get(3..).unwrap_or_default().trim();
            if value.contains(" -> ") {
                return Err(HarnessError::new(
                    "HARNESS_PATH_DENIED",
                    "renames must be expressed as an explicit delete and write",
                ));
            }
            let relative = normalize_relative_path(Path::new(value))?;
            if !workspace
                .allowed_path_prefixes
                .iter()
                .any(|prefix| relative.starts_with(prefix))
            {
                return Err(HarnessError::new(
                    "HARNESS_PATH_DENIED",
                    format!("changed path is outside the project boundary: {value}"),
                ));
            }
        }

        let source = fs::canonicalize(source_repository).map_err(|error| {
            HarnessError::new(
                "HARNESS_PUBLISH_FAILED",
                format!("local project could not be resolved: {error}"),
            )
        })?;
        if !source.is_dir() || source.starts_with(&self.root) {
            return Err(HarnessError::new(
                "HARNESS_PUBLISH_FAILED",
                "local project must be outside the temporary workspace root",
            ));
        }
        let source_status = self.run_git(Some(&source), &["status", "--porcelain=v1"])?;
        require_git_success(&source_status, "inspect local project before publish")?;
        if !source_status.stdout.is_empty() {
            return Err(HarnessError::new(
                "HARNESS_PUBLISH_CONFLICT",
                "local project changed while the harness was running",
            ));
        }
        let source_head = self.run_git(Some(&source), &["rev-parse", "HEAD"])?;
        require_git_success(&source_head, "read local project revision")?;
        if String::from_utf8_lossy(&source_head.stdout)
            .trim()
            .to_ascii_lowercase()
            != workspace.base_revision
        {
            return Err(HarnessError::new(
                "HARNESS_PUBLISH_CONFLICT",
                "local project revision changed while the harness was running",
            ));
        }

        require_git_success(
            &self.run_git(Some(workspace.path()), &["add", "--all"])?,
            "stage validated project changes",
        )?;
        let message = format!("mundusx: apply {task_id}");
        require_git_success(
            &self.run_git(
                Some(workspace.path()),
                &[
                    "-c",
                    "user.name=MundusX Harness",
                    "-c",
                    "user.email=harness@localhost",
                    "commit",
                    "-m",
                    &message,
                ],
            )?,
            "commit validated project changes",
        )?;
        let revision_output = self.run_git(Some(workspace.path()), &["rev-parse", "HEAD"])?;
        require_git_success(&revision_output, "read validated project revision")?;
        let revision = String::from_utf8_lossy(&revision_output.stdout)
            .trim()
            .to_ascii_lowercase();
        let workspace_text = git_path_text(workspace.path())?;
        require_git_success(
            &self.run_git(
                Some(&source),
                &["fetch", "--no-tags", &workspace_text, &revision],
            )?,
            "transfer validated project revision",
        )?;
        require_git_success(
            &self.run_git(Some(&source), &["merge", "--ff-only", &revision])?,
            "publish validated project revision",
        )?;
        Ok(Some(revision))
    }

    fn require_owned_workspace(&self, workspace: &PreparedWorkspace) -> Result<(), HarnessError> {
        ensure_direct_child(&self.root, workspace.path())?;
        let path = fs::canonicalize(workspace.path()).map_err(|error| {
            HarnessError::new(
                "HARNESS_PATH_DENIED",
                format!("workspace could not be resolved: {error}"),
            )
        })?;
        if !path.starts_with(&self.root) || path != workspace.path {
            return Err(HarnessError::new(
                "HARNESS_PATH_DENIED",
                "workspace is not owned by this manager",
            ));
        }
        Ok(())
    }

    fn run_git(
        &self,
        working_directory: Option<&Path>,
        args: &[&str],
    ) -> Result<Output, HarnessError> {
        let disabled_hooks = self.root.join(".disabled-hooks");
        let null_config = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let mut command = Command::new(&self.git_executable);
        command
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null_config)
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("-c")
            .arg(format!("core.hooksPath={}", disabled_hooks.display()))
            .arg("-c")
            .arg("credential.helper=")
            .args(args);
        if let Some(path) = working_directory {
            command.current_dir(path);
        }
        command.output().map_err(|error| {
            HarnessError::new(
                "HARNESS_WORKSPACE_PREPARE_FAILED",
                format!("failed to start trusted Git executable: {error}"),
            )
        })
    }
}

impl PreparedWorkspace {
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn base_revision(&self) -> &str {
        &self.base_revision
    }

    pub fn limits(&self) -> &WorkspaceLimits {
        &self.limits
    }

    /// Resolve an existing path and reject traversal, symlink, or junction
    /// escapes before a typed operation reads it.
    pub fn resolve_existing(&self, relative: &Path) -> Result<PathBuf, HarnessError> {
        let relative = normalize_relative_path(relative)?;
        self.require_allowed_prefix(&relative)?;
        reject_symlink_components(&self.path, &relative)?;
        let candidate = fs::canonicalize(self.path.join(&relative)).map_err(|error| {
            HarnessError::new(
                "HARNESS_PATH_DENIED",
                format!("path could not be resolved inside the workspace: {error}"),
            )
        })?;
        if !candidate.starts_with(&self.path) {
            return Err(HarnessError::new(
                "HARNESS_SYMLINK_ESCAPE",
                "resolved path escaped the workspace",
            ));
        }
        Ok(candidate)
    }

    /// Resolve a path that may not exist yet. Every existing ancestor is
    /// checked so patch creation cannot traverse a symlink out of the workspace.
    pub fn resolve_for_write(&self, relative: &Path) -> Result<PathBuf, HarnessError> {
        let relative = normalize_relative_path(relative)?;
        self.require_allowed_prefix(&relative)?;
        reject_symlink_components(&self.path, &relative)?;
        Ok(self.path.join(relative))
    }

    fn require_allowed_prefix(&self, relative: &Path) -> Result<(), HarnessError> {
        if self.allowed_path_prefixes.is_empty()
            || self
                .allowed_path_prefixes
                .iter()
                .any(|prefix| relative.starts_with(prefix))
        {
            return Ok(());
        }
        Err(HarnessError::new(
            "HARNESS_PATH_DENIED",
            "path is outside the assignment's allowed prefixes",
        ))
    }
}

fn validate_attempt_id(value: &str) -> Result<(), HarnessError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(HarnessError::new(
            "HARNESS_POLICY_DENIED",
            "attempt id must contain only ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn validate_revision(value: &str) -> Result<(), HarnessError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HarnessError::new(
            "HARNESS_BASE_REVISION_INVALID",
            "base revision must be a full 40-character Git commit id",
        ));
    }
    Ok(())
}

fn validate_limits(limits: &WorkspaceLimits) -> Result<(), HarnessError> {
    if limits.max_disk_mb == 0
        || limits.max_memory_mb == 0
        || limits.max_cpu_time_ms == 0
        || limits.max_wall_time_ms == 0
        || limits.max_cpu_time_ms > limits.max_wall_time_ms
    {
        return Err(HarnessError::new(
            "HARNESS_POLICY_DENIED",
            "workspace resource limits must be positive and CPU time cannot exceed wall time",
        ));
    }
    Ok(())
}

fn normalize_allowed_prefixes(prefixes: &[PathBuf]) -> Result<Vec<PathBuf>, HarnessError> {
    let mut normalized = prefixes
        .iter()
        .map(|prefix| normalize_relative_path(prefix))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf, HarnessError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(HarnessError::new(
            "HARNESS_PATH_DENIED",
            "workspace paths must be non-empty and relative",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            _ => {
                return Err(HarnessError::new(
                    "HARNESS_PATH_DENIED",
                    "workspace path contains traversal, root, or platform prefix components",
                ))
            }
        }
    }
    Ok(normalized)
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), HarnessError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(HarnessError::new(
                    "HARNESS_SYMLINK_ESCAPE",
                    "workspace path traverses a symlink",
                ))
            }
            Ok(_) => {
                let resolved = fs::canonicalize(&current).map_err(|error| {
                    HarnessError::new(
                        "HARNESS_PATH_DENIED",
                        format!("workspace path could not be resolved: {error}"),
                    )
                })?;
                if !resolved.starts_with(root) {
                    return Err(HarnessError::new(
                        "HARNESS_SYMLINK_ESCAPE",
                        "workspace path traverses a junction or reparse point outside the workspace",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(HarnessError::new(
                    "HARNESS_PATH_DENIED",
                    format!("workspace path metadata failed: {error}"),
                ))
            }
        }
    }
    Ok(())
}

fn ensure_direct_child(root: &Path, candidate: &Path) -> Result<(), HarnessError> {
    if candidate.parent() != Some(root) {
        return Err(HarnessError::new(
            "HARNESS_PATH_DENIED",
            "workspace directory is not a direct child of the configured root",
        ));
    }
    Ok(())
}

fn remove_workspace_directory(root: &Path, workspace: &Path) -> Result<(), HarnessError> {
    ensure_direct_child(root, workspace)?;
    let metadata = match fs::symlink_metadata(workspace) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(HarnessError::new(
                "HARNESS_CLEANUP_FAILED",
                format!("failed to inspect workspace before cleanup: {error}"),
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(HarnessError::new(
            "HARNESS_CLEANUP_FAILED",
            "refusing to recursively delete a non-directory or symlink workspace",
        ));
    }
    fs::remove_dir_all(workspace).map_err(|error| {
        HarnessError::new(
            "HARNESS_CLEANUP_FAILED",
            format!("failed to remove workspace: {error}"),
        )
    })
}

fn require_git_success(output: &Output, action: &str) -> Result<(), HarnessError> {
    if output.status.success() {
        return Ok(());
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    let diagnostic = diagnostic
        .lines()
        .next()
        .unwrap_or("Git returned a failure");
    Err(HarnessError::new(
        "HARNESS_WORKSPACE_PREPARE_FAILED",
        format!("failed to {action}: {diagnostic}"),
    ))
}

fn bounded_git_stdout(output: &Output, max_output_bytes: usize) -> Result<String, HarnessError> {
    if max_output_bytes == 0 || output.stdout.len() > max_output_bytes {
        return Err(HarnessError::new(
            "HARNESS_RESOURCE_EXHAUSTED",
            "repository operation exceeded its output budget",
        ));
    }
    String::from_utf8(output.stdout.clone()).map_err(|_| {
        HarnessError::new(
            "HARNESS_ARTIFACT_INVALID",
            "repository operation returned non-UTF-8 output",
        )
    })
}

fn git_path_text(path: &Path) -> Result<String, HarnessError> {
    let text = path.to_str().ok_or_else(|| {
        HarnessError::new(
            "HARNESS_PATH_DENIED",
            "workspace and source paths must be valid Unicode",
        )
    })?;
    if cfg!(windows) {
        if let Some(value) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(format!(r"\\{value}"));
        }
        if let Some(value) = text.strip_prefix(r"\\?\") {
            return Ok(value.to_string());
        }
    }
    Ok(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mundusx-harness-{label}-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).expect("create temporary test directory");
        path
    }

    fn git_executable() -> PathBuf {
        let locator = if cfg!(windows) { "where.exe" } else { "which" };
        let output = Command::new(locator)
            .arg("git")
            .output()
            .expect("locate Git for workspace tests");
        assert!(
            output.status.success(),
            "Git must be installed for workspace tests"
        );
        let path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .expect("Git locator returned a path")
            .trim()
            .to_string();
        PathBuf::from(path)
    }

    fn run_git(git: &Path, directory: &Path, args: &[&str]) -> Output {
        Command::new(git)
            .current_dir(directory)
            .args(args)
            .output()
            .expect("run Git fixture command")
    }

    fn fixture_repository(git: &Path) -> (PathBuf, String) {
        let repository = temporary_directory("repository");
        assert!(run_git(git, &repository, &["init"]).status.success());
        assert!(
            run_git(git, &repository, &["config", "user.name", "Harness Test"])
                .status
                .success()
        );
        assert!(run_git(
            git,
            &repository,
            &["config", "user.email", "harness@example.invalid"]
        )
        .status
        .success());
        fs::create_dir_all(repository.join("src")).expect("create fixture source folder");
        fs::write(
            repository.join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("write fixture source");
        assert!(run_git(git, &repository, &["add", "."]).status.success());
        assert!(run_git(git, &repository, &["commit", "-m", "fixture"])
            .status
            .success());
        let head = run_git(git, &repository, &["rev-parse", "HEAD"]);
        assert!(head.status.success());
        let revision = String::from_utf8_lossy(&head.stdout).trim().to_string();
        (repository, revision)
    }

    fn request(repository: &Path, revision: &str, attempt_id: &str) -> WorkspaceRequest {
        WorkspaceRequest {
            attempt_id: attempt_id.to_string(),
            source_repository: repository.to_path_buf(),
            base_revision: revision.to_string(),
            allowed_path_prefixes: vec![PathBuf::from("src")],
            limits: WorkspaceLimits::default(),
        }
    }

    #[test]
    fn prepares_unique_immutable_workspaces_and_cleans_them_idempotently() {
        let git = git_executable();
        let (repository, revision) = fixture_repository(&git);
        let root = temporary_directory("root");
        let manager = WorkspaceManager::new(root.clone(), git).expect("create workspace manager");

        let first = manager
            .prepare(request(&repository, &revision, "attempt_one"))
            .expect("prepare first workspace");
        let second = manager
            .prepare(request(&repository, &revision, "attempt_two"))
            .expect("prepare second workspace");

        assert_ne!(first.path(), second.path());
        assert_eq!(first.base_revision(), revision);
        assert_eq!(
            fs::read_to_string(first.resolve_existing(Path::new("src/lib.rs")).unwrap()).unwrap(),
            "pub fn answer() -> u8 { 42 }\n"
        );
        fs::write(
            first.resolve_for_write(Path::new("src/lib.rs")).unwrap(),
            "changed\n",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(second.resolve_existing(Path::new("src/lib.rs")).unwrap()).unwrap(),
            "pub fn answer() -> u8 { 42 }\n"
        );

        manager.cleanup(&first).expect("cleanup first workspace");
        manager.cleanup(&first).expect("cleanup remains idempotent");
        manager.cleanup(&second).expect("cleanup second workspace");
        fs::remove_dir_all(repository).expect("cleanup fixture repository");
        fs::remove_dir_all(root).expect("cleanup fixture root");
    }

    #[test]
    fn publishes_only_validated_workspace_changes_back_to_local_project() {
        let git = git_executable();
        let (repository, revision) = fixture_repository(&git);
        let root = temporary_directory("publish-root");
        let manager = WorkspaceManager::new(root.clone(), git.clone()).expect("create manager");
        let workspace = manager
            .prepare(request(&repository, &revision, "attempt_publish"))
            .expect("prepare workspace");
        fs::write(
            workspace.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 43 }\n",
        )
        .expect("edit workspace");
        let published = manager
            .publish_validated_changes(&workspace, &repository, "task-local")
            .expect("publish validated changes")
            .expect("new revision");
        assert_ne!(published, revision);
        assert_eq!(
            fs::read_to_string(repository.join("src/lib.rs")).expect("read project"),
            "pub fn answer() -> u8 { 43 }\n"
        );
        manager.cleanup(&workspace).expect("cleanup workspace");
        fs::remove_dir_all(repository).expect("remove repository fixture");
        fs::remove_dir_all(root).expect("remove workspace fixture");
    }

    #[test]
    fn rejects_traversal_absolute_paths_and_out_of_scope_prefixes() {
        let git = git_executable();
        let (repository, revision) = fixture_repository(&git);
        let root = temporary_directory("paths");
        let manager = WorkspaceManager::new(root.clone(), git).unwrap();
        let workspace = manager
            .prepare(request(&repository, &revision, "attempt_paths"))
            .unwrap();

        for denied in [Path::new("../secret"), Path::new("tests/outside.rs")] {
            let error = workspace.resolve_for_write(denied).unwrap_err();
            assert_eq!(error.code, "HARNESS_PATH_DENIED");
        }
        let absolute = if cfg!(windows) {
            Path::new("C:\\Windows\\System32")
        } else {
            Path::new("/etc/passwd")
        };
        assert_eq!(
            workspace.resolve_existing(absolute).unwrap_err().code,
            "HARNESS_PATH_DENIED"
        );

        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let git = git_executable();
        let (repository, revision) = fixture_repository(&git);
        let root = temporary_directory("symlink-root");
        let outside = temporary_directory("symlink-outside");
        fs::write(outside.join("secret"), "not visible").unwrap();
        let manager = WorkspaceManager::new(root.clone(), git).unwrap();
        let workspace = manager
            .prepare(request(&repository, &revision, "attempt_symlink"))
            .unwrap();
        symlink(&outside, workspace.path().join("src/escape")).unwrap();

        assert_eq!(
            workspace
                .resolve_existing(Path::new("src/escape/secret"))
                .unwrap_err()
                .code,
            "HARNESS_SYMLINK_ESCAPE"
        );

        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_symlink_escape_when_supported_by_the_host() {
        use std::os::windows::fs::symlink_file;

        let git = git_executable();
        let (repository, revision) = fixture_repository(&git);
        let root = temporary_directory("symlink-root");
        let outside = temporary_directory("symlink-outside");
        let outside_file = outside.join("secret");
        fs::write(&outside_file, "not visible").unwrap();
        let manager = WorkspaceManager::new(root.clone(), git).unwrap();
        let workspace = manager
            .prepare(request(&repository, &revision, "attempt_symlink"))
            .unwrap();
        let link = workspace.path().join("src/escape");

        match symlink_file(&outside_file, &link) {
            Ok(()) => assert_eq!(
                workspace
                    .resolve_existing(Path::new("src/escape"))
                    .unwrap_err()
                    .code,
                "HARNESS_SYMLINK_ESCAPE"
            ),
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(1_314) =>
            {
                // Windows hosts without Developer Mode cannot create a symlink.
                // Production rejection is still enforced by metadata and
                // canonical-path checks; the Unix test exercises the escape.
            }
            Err(error) => panic!("unexpected symlink fixture failure: {error}"),
        }

        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
