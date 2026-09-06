//! Typed Coding Harness v1 repository and validation operations.
//!
//! Model output is never executable authority. Repository edits are
//! content-addressed file replacements, and validation runs only a
//! server-owned profile through a bounded process or pinned sandbox runtime.

use crate::harness::{HarnessError, PreparedWorkspace, WorkspaceManager};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 1_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileReadResult {
    pub path: String,
    pub content: String,
    pub sha256: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub excerpt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileReplacement {
    pub path: String,
    pub expected_sha256: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PatchResult {
    pub path: String,
    pub previous_sha256: Option<String>,
    pub sha256: String,
    pub bytes_written: usize,
    pub idempotent_replay: bool,
}

#[derive(Debug, Default)]
pub struct RepositoryToolRunner {
    applied: Mutex<HashMap<String, (String, PatchResult)>>,
}

impl RepositoryToolRunner {
    pub fn read_file(
        &self,
        workspace: &PreparedWorkspace,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<FileReadResult, HarnessError> {
        if max_bytes == 0 || max_bytes > MAX_FILE_BYTES {
            return Err(error(
                "HARNESS_POLICY_DENIED",
                "file read budget is outside the allowed range",
            ));
        }
        let path = workspace.resolve_existing(relative)?;
        if !path.is_file() {
            return Err(error("HARNESS_PATH_DENIED", "requested path is not a file"));
        }
        let metadata = fs::metadata(&path).map_err(path_error)?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(error(
                "HARNESS_RESOURCE_EXHAUSTED",
                "file exceeds the harness file-size limit",
            ));
        }
        let bytes = fs::read(&path).map_err(path_error)?;
        if bytes.contains(&0) {
            return Err(error(
                "HARNESS_ARTIFACT_INVALID",
                "binary files are not readable through the text tool",
            ));
        }
        let digest = sha256_hex(&bytes);
        let truncated = bytes.len() > max_bytes;
        let visible = &bytes[..bytes.len().min(max_bytes)];
        let content = String::from_utf8(visible.to_vec()).map_err(|_| {
            error(
                "HARNESS_ARTIFACT_INVALID",
                "file content is not valid UTF-8 text",
            )
        })?;
        Ok(FileReadResult {
            path: slash_path(relative),
            content: redact_sensitive_lines(&content),
            sha256: digest,
            truncated,
        })
    }

    pub fn search(
        &self,
        workspace: &PreparedWorkspace,
        relative_root: &Path,
        query: &str,
        max_results: usize,
        max_output_bytes: usize,
    ) -> Result<Vec<SearchMatch>, HarnessError> {
        if query.trim().is_empty()
            || query.len() > 512
            || max_results == 0
            || max_results > MAX_SEARCH_RESULTS
            || max_output_bytes == 0
        {
            return Err(error(
                "HARNESS_POLICY_DENIED",
                "search query or result budget is invalid",
            ));
        }
        let root = workspace.resolve_existing(relative_root)?;
        if !root.is_dir() {
            return Err(error(
                "HARNESS_PATH_DENIED",
                "search root is not a directory",
            ));
        }
        let mut pending = vec![root];
        let mut matches = Vec::new();
        let mut output_bytes = 0usize;
        while let Some(directory) = pending.pop() {
            let mut entries = fs::read_dir(&directory)
                .map_err(path_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(path_error)?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).map_err(path_error)?;
                if metadata.file_type().is_symlink() {
                    return Err(error(
                        "HARNESS_SYMLINK_ESCAPE",
                        "search encountered a symlink and failed closed",
                    ));
                }
                if metadata.is_dir() {
                    if path.file_name().is_some_and(|name| name == ".git") {
                        continue;
                    }
                    pending.push(path);
                    continue;
                }
                if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
                    continue;
                }
                let relative = path.strip_prefix(workspace.path()).map_err(|_| {
                    error("HARNESS_PATH_DENIED", "search path escaped the workspace")
                })?;
                let resolved = workspace.resolve_existing(relative)?;
                let bytes = fs::read(resolved).map_err(path_error)?;
                if bytes.contains(&0) {
                    continue;
                }
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                for (index, line) in text.lines().enumerate() {
                    if !line.contains(query) {
                        continue;
                    }
                    let excerpt = redact_sensitive_lines(line)
                        .chars()
                        .take(500)
                        .collect::<String>();
                    let size = relative.as_os_str().len() + excerpt.len() + 32;
                    if matches.len() >= max_results
                        || output_bytes.saturating_add(size) > max_output_bytes
                    {
                        return Ok(matches);
                    }
                    output_bytes += size;
                    matches.push(SearchMatch {
                        path: slash_path(relative),
                        line: index + 1,
                        excerpt,
                    });
                }
            }
        }
        Ok(matches)
    }

    pub fn apply_replacement(
        &self,
        workspace: &PreparedWorkspace,
        idempotency_key: &str,
        replacement: &FileReplacement,
    ) -> Result<PatchResult, HarnessError> {
        validate_idempotency_key(idempotency_key)?;
        if replacement.content.len() > MAX_FILE_BYTES || replacement.content.as_bytes().contains(&0)
        {
            return Err(error(
                "HARNESS_ARTIFACT_INVALID",
                "replacement must be bounded UTF-8 source text",
            ));
        }
        let relative = Path::new(&replacement.path);
        let path = workspace.resolve_for_write(relative)?;
        let input_digest = replacement_digest(replacement);
        {
            let applied = self.applied.lock().map_err(|_| {
                error(
                    "HARNESS_STATE_CONFLICT",
                    "patch idempotency state is unavailable",
                )
            })?;
            if let Some((previous_input, previous_result)) = applied.get(idempotency_key) {
                if previous_input != &input_digest {
                    return Err(error(
                        "HARNESS_IDEMPOTENCY_CONFLICT",
                        "idempotency key was reused with different patch input",
                    ));
                }
                let mut result = previous_result.clone();
                result.idempotent_replay = true;
                return Ok(result);
            }
        }

        let previous = match fs::read(&path) {
            Ok(bytes) => Some(sha256_hex(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(path_error(error)),
        };
        match (&previous, &replacement.expected_sha256) {
            (Some(actual), Some(expected)) if actual.eq_ignore_ascii_case(expected) => {}
            (None, None) => {}
            (Some(_), None) => {
                return Err(error(
                    "HARNESS_ARTIFACT_INVALID",
                    "editing an existing file requires its expected SHA-256",
                ))
            }
            _ => {
                return Err(error(
                    "HARNESS_ARTIFACT_INVALID",
                    "file changed since the model read it",
                ))
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(path_error)?;
            let relative_parent = parent
                .strip_prefix(workspace.path())
                .map_err(|_| error("HARNESS_PATH_DENIED", "patch parent escaped the workspace"))?;
            workspace.resolve_for_write(relative_parent)?;
        }
        let temporary = path.with_extension(format!(
            "{}.{}.partial",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("file"),
            uuid::Uuid::new_v4().simple()
        ));
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(path_error)?;
            file.write_all(replacement.content.as_bytes())
                .map_err(path_error)?;
            file.sync_all().map_err(path_error)?;
        }
        if path.exists() {
            fs::remove_file(&path).map_err(path_error)?;
        }
        fs::rename(&temporary, &path).map_err(path_error)?;
        let result = PatchResult {
            path: replacement.path.clone(),
            previous_sha256: previous,
            sha256: sha256_hex(replacement.content.as_bytes()),
            bytes_written: replacement.content.len(),
            idempotent_replay: false,
        };
        self.applied
            .lock()
            .map_err(|_| {
                error(
                    "HARNESS_STATE_CONFLICT",
                    "patch idempotency state is unavailable",
                )
            })?
            .insert(idempotency_key.to_string(), (input_digest, result.clone()));
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Disabled,
    Allowed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationIsolation {
    TrustedHybrid,
    DockerSandbox {
        runtime: PathBuf,
        image_digest: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationProfile {
    pub profile_id: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub network: NetworkPolicy,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub max_memory_mb: u32,
    pub max_cpu_time_ms: u64,
    pub max_processes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub profile_id: String,
    pub status: String,
    pub code: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: bool,
    pub output_sha256: String,
}

#[derive(Clone, Debug)]
pub struct ValidationRunner {
    isolation: ValidationIsolation,
}

impl ValidationRunner {
    pub fn new(isolation: ValidationIsolation) -> Result<Self, HarnessError> {
        match &isolation {
            ValidationIsolation::TrustedHybrid => {}
            ValidationIsolation::DockerSandbox {
                runtime,
                image_digest,
            } => {
                if !runtime.is_absolute() || !image_digest.contains("@sha256:") {
                    return Err(error(
                        "HARNESS_POLICY_DENIED",
                        "sandbox runtime must be absolute and its image must be digest pinned",
                    ));
                }
            }
        }
        Ok(Self { isolation })
    }

    pub fn run(
        &self,
        workspace: &PreparedWorkspace,
        profile: &ValidationProfile,
        cancelled: &AtomicBool,
    ) -> Result<ValidationResult, HarnessError> {
        validate_profile(profile, &self.isolation)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(error(
                "HARNESS_CANCELLED",
                "validation was cancelled before start",
            ));
        }
        let working_directory = workspace.resolve_existing(&profile.working_directory)?;
        if !working_directory.is_dir() {
            return Err(error(
                "HARNESS_VALIDATION_PROFILE_DENIED",
                "validation working directory is not a directory",
            ));
        }
        let mut command_plan = self.build_command(workspace, profile, &working_directory)?;
        let command = &mut command_plan.command;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_platform_command(command, profile);
        let started = Instant::now();
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error_value) => {
                command_plan.cleanup.run();
                return Err(error(
                    "HARNESS_VALIDATION_PROFILE_DENIED",
                    format!("validation executable could not start: {error_value}"),
                ));
            }
        };
        let process_group = match ProcessGroup::attach(&child, profile) {
            Ok(group) => group,
            Err(error_value) => {
                let _ = child.kill();
                let _ = child.wait();
                command_plan.cleanup.run();
                return Err(error_value);
            }
        };
        let stdout = child.stdout.take().ok_or_else(|| {
            error(
                "HARNESS_STATE_CONFLICT",
                "validation stdout pipe was unavailable",
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            error(
                "HARNESS_STATE_CONFLICT",
                "validation stderr pipe was unavailable",
            )
        })?;
        let stdout_reader = spawn_bounded_reader(stdout, profile.max_output_bytes);
        let stderr_reader = spawn_bounded_reader(stderr, profile.max_output_bytes);
        let deadline = started + Duration::from_millis(profile.timeout_ms);
        let mut terminal_code = "HARNESS_VALIDATION_FAILED";
        let status: ExitStatus;
        loop {
            if let Some(value) = child.try_wait().map_err(|error_value| {
                error(
                    "HARNESS_STATE_CONFLICT",
                    format!("validation process status failed: {error_value}"),
                )
            })? {
                status = value;
                break;
            }
            if cancelled.load(Ordering::Acquire) {
                terminal_code = "HARNESS_CANCELLED";
                process_group.terminate();
                status = child.wait().map_err(process_wait_error)?;
                break;
            }
            if Instant::now() >= deadline {
                terminal_code = "HARNESS_TIMEOUT";
                process_group.terminate();
                status = child.wait().map_err(process_wait_error)?;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(process_group);
        command_plan.cleanup.run();
        let (stdout_bytes, stdout_truncated) = join_reader(stdout_reader)?;
        let (stderr_bytes, stderr_truncated) = join_reader(stderr_reader)?;
        let stdout = redact_sensitive_lines(&String::from_utf8_lossy(&stdout_bytes));
        let stderr = redact_sensitive_lines(&String::from_utf8_lossy(&stderr_bytes));
        let output_truncated = stdout_truncated || stderr_truncated;
        let (result_status, code) = if terminal_code == "HARNESS_CANCELLED" {
            ("cancelled", terminal_code)
        } else if terminal_code == "HARNESS_TIMEOUT" {
            ("failed", terminal_code)
        } else if status.success() && !output_truncated {
            ("passed", "OK")
        } else if output_truncated {
            ("failed", "HARNESS_RESOURCE_EXHAUSTED")
        } else {
            ("failed", "HARNESS_VALIDATION_FAILED")
        };
        let combined_digest =
            sha256_hex([stdout.as_bytes(), stderr.as_bytes()].concat().as_slice());
        Ok(ValidationResult {
            profile_id: profile.profile_id.clone(),
            status: result_status.to_string(),
            code: code.to_string(),
            exit_code: status.code(),
            duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            stdout,
            stderr,
            output_truncated,
            output_sha256: combined_digest,
        })
    }

    fn build_command(
        &self,
        workspace: &PreparedWorkspace,
        profile: &ValidationProfile,
        working_directory: &Path,
    ) -> Result<CommandPlan, HarnessError> {
        match &self.isolation {
            ValidationIsolation::TrustedHybrid => {
                let mut command = Command::new(&profile.executable);
                command.current_dir(working_directory).env_clear();
                for (name, value) in &profile.environment {
                    command.env(name, value);
                }
                command.args(&profile.arguments);
                Ok(CommandPlan {
                    command,
                    cleanup: SandboxCleanup::None,
                })
            }
            ValidationIsolation::DockerSandbox {
                runtime,
                image_digest,
            } => {
                let source = workspace.path().to_str().ok_or_else(|| {
                    error("HARNESS_PATH_DENIED", "workspace path is not valid Unicode")
                })?;
                if source.contains(',') {
                    return Err(error(
                        "HARNESS_PATH_DENIED",
                        "sandbox workspace path cannot contain a comma",
                    ));
                }
                let relative_working =
                    working_directory
                        .strip_prefix(workspace.path())
                        .map_err(|_| {
                            error(
                                "HARNESS_PATH_DENIED",
                                "validation directory escaped workspace",
                            )
                        })?;
                let container_working = if relative_working.as_os_str().is_empty() {
                    "/workspace".to_string()
                } else {
                    format!("/workspace/{}", slash_path(relative_working))
                };
                let mut command = Command::new(runtime);
                let cidfile = workspace.path().join(format!(
                    ".mundusx-container-{}.cid",
                    uuid::Uuid::new_v4().simple()
                ));
                let cidfile_text = cidfile.to_str().ok_or_else(|| {
                    error(
                        "HARNESS_PATH_DENIED",
                        "container id path is not valid Unicode",
                    )
                })?;
                command.env_clear().args([
                    "run",
                    "--rm",
                    "--read-only",
                    "--network",
                    "none",
                    "--cidfile",
                    cidfile_text,
                    "--pids-limit",
                    profile.max_processes.to_string().as_str(),
                    "--memory",
                    format!("{}m", profile.max_memory_mb).as_str(),
                    "--cpus",
                    "1",
                    "--tmpfs",
                    "/tmp:rw,noexec,nosuid,size=64m",
                    "--mount",
                    format!("type=bind,source={source},target=/workspace").as_str(),
                    "--workdir",
                    container_working.as_str(),
                ]);
                for (name, value) in &profile.environment {
                    command.arg("--env").arg(format!("{name}={value}"));
                }
                command
                    .arg(image_digest)
                    .arg(&profile.executable)
                    .args(&profile.arguments);
                Ok(CommandPlan {
                    command,
                    cleanup: SandboxCleanup::Docker {
                        runtime: runtime.clone(),
                        cidfile,
                    },
                })
            }
        }
    }
}

struct CommandPlan {
    command: Command,
    cleanup: SandboxCleanup,
}

enum SandboxCleanup {
    None,
    Docker { runtime: PathBuf, cidfile: PathBuf },
}

impl SandboxCleanup {
    fn run(&self) {
        let Self::Docker { runtime, cidfile } = self else {
            return;
        };
        if let Ok(container_id) = fs::read_to_string(cidfile) {
            let container_id = container_id.trim();
            if !container_id.is_empty()
                && container_id
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            {
                let _ = Command::new(runtime)
                    .env_clear()
                    .args(["kill", container_id])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        let _ = fs::remove_file(cidfile);
    }
}

fn validate_profile(
    profile: &ValidationProfile,
    isolation: &ValidationIsolation,
) -> Result<(), HarnessError> {
    if profile.profile_id.is_empty()
        || profile.profile_id.len() > 96
        || !profile
            .profile_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || profile.timeout_ms == 0
        || profile.max_output_bytes == 0
        || profile.max_output_bytes > 4 * 1024 * 1024
        || profile.max_memory_mb == 0
        || profile.max_cpu_time_ms == 0
        || profile.max_processes == 0
    {
        return Err(error(
            "HARNESS_VALIDATION_PROFILE_DENIED",
            "validation profile identifiers or limits are invalid",
        ));
    }
    if profile
        .arguments
        .iter()
        .any(|argument| argument.as_bytes().contains(&0))
    {
        return Err(error(
            "HARNESS_VALIDATION_PROFILE_DENIED",
            "validation arguments contain a null byte",
        ));
    }
    for (name, value) in &profile.environment {
        let safe_name = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
        let upper = name.to_ascii_uppercase();
        if !safe_name
            || ["TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "API_KEY"]
                .iter()
                .any(|marker| upper.contains(marker))
            || value.as_bytes().contains(&0)
        {
            return Err(error(
                "HARNESS_VALIDATION_PROFILE_DENIED",
                "validation environment is not in the non-secret allowlist",
            ));
        }
    }
    match isolation {
        ValidationIsolation::TrustedHybrid => {
            if !profile.executable.is_absolute() || profile.network != NetworkPolicy::Allowed {
                return Err(error(
                    "HARNESS_NETWORK_DENIED",
                    "trusted hybrid execution requires an absolute executable and explicit network policy; use sandbox mode to guarantee disabled network",
                ));
            }
        }
        ValidationIsolation::DockerSandbox { .. } => {
            if !profile.executable.is_absolute() || profile.network != NetworkPolicy::Disabled {
                return Err(error(
                    "HARNESS_NETWORK_DENIED",
                    "sandbox validation requires an absolute container executable and disabled network",
                ));
            }
        }
    }
    Ok(())
}

fn configure_platform_command(command: &mut Command, profile: &ValidationProfile) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let memory_bytes = u64::from(profile.max_memory_mb) * 1024 * 1024;
        let cpu_seconds = profile.max_cpu_time_ms.div_ceil(1_000).max(1);
        command.process_group(0);
        unsafe {
            command.pre_exec(move || {
                let memory = libc::rlimit {
                    rlim_cur: memory_bytes,
                    rlim_max: memory_bytes,
                };
                let cpu = libc::rlimit {
                    rlim_cur: cpu_seconds,
                    rlim_max: cpu_seconds,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0
                    || libc::setrlimit(libc::RLIMIT_CPU, &cpu) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    let _ = (command, profile);
}

struct ProcessGroup {
    #[cfg(unix)]
    pid: u32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl ProcessGroup {
    fn attach(
        child: &std::process::Child,
        profile: &ValidationProfile,
    ) -> Result<Self, HarnessError> {
        #[cfg(windows)]
        {
            use std::mem::{size_of, zeroed};
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_JOB_TIME,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };

            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() {
                return Err(process_limit_error("failed to create Windows job object"));
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_JOB_TIME;
            limits.JobMemoryLimit = profile.max_memory_mb as usize * 1024 * 1024;
            limits.BasicLimitInformation.PerJobUserTimeLimit =
                (profile.max_cpu_time_ms.saturating_mul(10_000)).min(i64::MAX as u64) as i64;
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const core::ffi::c_void,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            let process = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
            let assigned = unsafe { AssignProcessToJobObject(job, process) };
            if configured == 0 || assigned == 0 {
                unsafe { CloseHandle(job) };
                return Err(process_limit_error(
                    "failed to configure or assign Windows job object",
                ));
            }
            return Ok(Self { job });
        }
        #[cfg(unix)]
        {
            let _ = profile;
            Ok(Self { pid: child.id() })
        }
    }

    fn terminate(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

fn spawn_bounded_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<Result<(Vec<u8>, bool), std::io::Error>> {
    thread::spawn(move || {
        let mut retained = Vec::with_capacity(limit.min(64 * 1024));
        let mut buffer = [0u8; 8 * 1024];
        let mut truncated = false;
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let remaining = limit.saturating_sub(retained.len());
            let keep = count.min(remaining);
            retained.extend_from_slice(&buffer[..keep]);
            truncated |= keep < count;
        }
        Ok((retained, truncated))
    })
}

fn join_reader(
    handle: thread::JoinHandle<Result<(Vec<u8>, bool), std::io::Error>>,
) -> Result<(Vec<u8>, bool), HarnessError> {
    handle
        .join()
        .map_err(|_| {
            error(
                "HARNESS_STATE_CONFLICT",
                "validation output reader panicked",
            )
        })?
        .map_err(|error_value| {
            error(
                "HARNESS_STATE_CONFLICT",
                format!("validation output read failed: {error_value}"),
            )
        })
}

fn redact_sensitive_lines(value: &str) -> String {
    value
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "authorization:",
                "api_key=",
                "apikey=",
                "password=",
                "access_token=",
                "secret_key=",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
            {
                "[REDACTED]".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_idempotency_key(value: &str) -> Result<(), HarnessError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(error(
            "HARNESS_IDEMPOTENCY_REQUIRED",
            "patch operation requires a bounded idempotency key",
        ));
    }
    Ok(())
}

fn replacement_digest(replacement: &FileReplacement) -> String {
    let mut hasher = Sha256::new();
    hasher.update(replacement.path.as_bytes());
    hasher.update([0]);
    if let Some(expected) = &replacement.expected_sha256 {
        hasher.update(expected.as_bytes());
    }
    hasher.update([0]);
    hasher.update(replacement.content.as_bytes());
    hex::encode(hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn error(code: &'static str, message: impl Into<String>) -> HarnessError {
    HarnessError {
        code,
        message: message.into(),
    }
}

fn path_error(error_value: std::io::Error) -> HarnessError {
    error(
        "HARNESS_PATH_DENIED",
        format!("repository operation failed: {error_value}"),
    )
}

fn process_wait_error(error_value: std::io::Error) -> HarnessError {
    error(
        "HARNESS_STATE_CONFLICT",
        format!("validation process wait failed: {error_value}"),
    )
}

fn process_limit_error(message: &str) -> HarnessError {
    error("HARNESS_RESOURCE_EXHAUSTED", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{WorkspaceLimits, WorkspaceRequest};
    use std::sync::atomic::AtomicBool;
    use uuid::Uuid;

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mundusx-harness-tools-{label}-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn locate(name: &str) -> PathBuf {
        let locator = if cfg!(windows) { "where.exe" } else { "which" };
        let output = Command::new(locator).arg(name).output().unwrap();
        assert!(
            output.status.success(),
            "required executable {name} was not found"
        );
        PathBuf::from(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap()
                .trim(),
        )
    }

    fn git(directory: &Path, args: &[&str]) -> std::process::Output {
        Command::new(locate("git"))
            .current_dir(directory)
            .args(args)
            .output()
            .unwrap()
    }

    fn workspace() -> (WorkspaceManager, PreparedWorkspace, PathBuf, PathBuf) {
        let repository = temporary_directory("repository");
        assert!(git(&repository, &["init"]).status.success());
        assert!(git(&repository, &["config", "user.name", "Harness Test"])
            .status
            .success());
        assert!(git(
            &repository,
            &["config", "user.email", "harness@example.invalid"]
        )
        .status
        .success());
        fs::create_dir_all(repository.join("src")).unwrap();
        fs::write(
            repository.join("src/lib.rs"),
            "pub fn answer() -> u8 { 41 }\n",
        )
        .unwrap();
        assert!(git(&repository, &["add", "."]).status.success());
        assert!(git(&repository, &["commit", "-m", "fixture"])
            .status
            .success());
        let head = git(&repository, &["rev-parse", "HEAD"]);
        let revision = String::from_utf8_lossy(&head.stdout).trim().to_string();
        let root = temporary_directory("root");
        let manager = WorkspaceManager::new(root.clone(), locate("git")).unwrap();
        let prepared = manager
            .prepare(WorkspaceRequest {
                attempt_id: "tools_attempt".to_string(),
                source_repository: repository.clone(),
                base_revision: revision,
                allowed_path_prefixes: vec![PathBuf::from("src")],
                limits: WorkspaceLimits::default(),
            })
            .unwrap();
        (manager, prepared, repository, root)
    }

    #[test]
    fn typed_repository_tools_read_search_patch_diff_and_replay_safely() {
        let (manager, workspace, repository, root) = workspace();
        let tools = RepositoryToolRunner::default();
        let read = tools
            .read_file(&workspace, Path::new("src/lib.rs"), 10_000)
            .unwrap();
        assert!(read.content.contains("41"));
        let matches = tools
            .search(&workspace, Path::new("src"), "answer", 10, 10_000)
            .unwrap();
        assert_eq!(matches.len(), 1);
        let replacement = FileReplacement {
            path: "src/lib.rs".to_string(),
            expected_sha256: Some(read.sha256),
            content: "pub fn answer() -> u8 { 42 }\n".to_string(),
        };
        let first = tools
            .apply_replacement(&workspace, "patch_1", &replacement)
            .unwrap();
        assert!(!first.idempotent_replay);
        let replay = tools
            .apply_replacement(&workspace, "patch_1", &replacement)
            .unwrap();
        assert!(replay.idempotent_replay);
        assert!(manager
            .repository_diff(&workspace, 100_000)
            .unwrap()
            .contains("42"));
        let conflicting = FileReplacement {
            content: "different".to_string(),
            ..replacement
        };
        assert_eq!(
            tools
                .apply_replacement(&workspace, "patch_1", &conflicting)
                .unwrap_err()
                .code,
            "HARNESS_IDEMPOTENCY_CONFLICT"
        );
        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn helper_profile(mode: &str, timeout_ms: u64, max_output_bytes: usize) -> ValidationProfile {
        let mut environment = BTreeMap::new();
        environment.insert("MUNDUSX_HARNESS_TEST_HELPER".to_string(), mode.to_string());
        ValidationProfile {
            profile_id: "test-profile".to_string(),
            executable: std::env::current_exe().unwrap(),
            arguments: vec![
                "--exact".to_string(),
                "harness_tools::tests::validation_helper_process".to_string(),
                "--nocapture".to_string(),
            ],
            working_directory: PathBuf::from("src"),
            environment,
            network: NetworkPolicy::Allowed,
            timeout_ms,
            max_output_bytes,
            max_memory_mb: 512,
            max_cpu_time_ms: timeout_ms.max(1),
            max_processes: 8,
        }
    }

    #[test]
    fn bounded_validation_reports_success_redaction_and_output_limits() {
        let (manager, workspace, repository, root) = workspace();
        let runner = ValidationRunner::new(ValidationIsolation::TrustedHybrid).unwrap();
        let cancelled = AtomicBool::new(false);
        let passed = runner
            .run(
                &workspace,
                &helper_profile("pass", 5_000, 20_000),
                &cancelled,
            )
            .unwrap();
        assert_eq!(passed.status, "passed");
        assert!(passed.stdout.contains("validation-ok"));
        assert!(passed.stdout.contains("[REDACTED]"));

        let bounded = runner
            .run(&workspace, &helper_profile("large", 5_000, 128), &cancelled)
            .unwrap();
        assert_eq!(bounded.code, "HARNESS_RESOURCE_EXHAUSTED");
        assert!(bounded.output_truncated);
        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sandbox_plan_is_digest_pinned_network_disabled_and_resource_bounded() {
        let (manager, workspace, repository, root) = workspace();
        let runtime = if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\Docker\docker.exe")
        } else {
            PathBuf::from("/usr/bin/docker")
        };
        let runner = ValidationRunner::new(ValidationIsolation::DockerSandbox {
            runtime,
            image_digest: "registry.invalid/harness@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        })
        .unwrap();
        let executable = if cfg!(windows) {
            PathBuf::from(r"C:\usr\bin\cargo.exe")
        } else {
            PathBuf::from("/usr/bin/cargo")
        };
        let profile = ValidationProfile {
            executable,
            network: NetworkPolicy::Disabled,
            ..helper_profile("pass", 5_000, 20_000)
        };
        let working = workspace.resolve_existing(Path::new("src")).unwrap();
        let plan = runner
            .build_command(&workspace, &profile, &working)
            .unwrap();
        let arguments = plan
            .command
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--network", "none"]));
        assert!(arguments.iter().any(|value| value == "--read-only"));
        assert!(arguments.iter().any(|value| value == "--pids-limit"));
        assert!(arguments
            .iter()
            .any(|value| value.contains("@sha256:aaaaaaaa")));
        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn timeout_terminates_the_validation_process_tree() {
        let (manager, workspace, repository, root) = workspace();
        let marker = root.join("descendant-survived");
        let runner = ValidationRunner::new(ValidationIsolation::TrustedHybrid).unwrap();
        let cancelled = AtomicBool::new(false);
        let mode = format!("tree:{}", marker.display());
        let timed_out = runner
            .run(&workspace, &helper_profile(&mode, 150, 20_000), &cancelled)
            .unwrap();
        assert_eq!(timed_out.code, "HARNESS_TIMEOUT");
        thread::sleep(Duration::from_millis(1_200));
        assert!(
            !marker.exists(),
            "descendant process survived timeout cleanup"
        );
        manager.cleanup(&workspace).unwrap();
        fs::remove_dir_all(repository).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn validation_helper_process() {
        let Ok(mode) = std::env::var("MUNDUSX_HARNESS_TEST_HELPER") else {
            return;
        };
        match mode.as_str() {
            "pass" => {
                println!("validation-ok");
                println!("api_key=must-not-leak");
            }
            "large" => println!("{}", "x".repeat(8_192)),
            value if value.starts_with("tree:") => {
                let marker = value.trim_start_matches("tree:");
                let mut child = Command::new(std::env::current_exe().unwrap());
                child
                    .args([
                        "--exact",
                        "harness_tools::tests::validation_helper_process",
                        "--nocapture",
                    ])
                    .env("MUNDUSX_HARNESS_TEST_HELPER", format!("leaf:{marker}"));
                child.spawn().unwrap();
                thread::sleep(Duration::from_secs(10));
            }
            value if value.starts_with("leaf:") => {
                thread::sleep(Duration::from_millis(800));
                fs::write(value.trim_start_matches("leaf:"), "survived").unwrap();
            }
            _ => panic!("unknown helper mode"),
        }
    }
}
