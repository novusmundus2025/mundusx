//! Signed control-plane client for the user-owned Coding Harness v1 runner.

use crate::config::RunnerConfig;
use crate::harness::{HarnessError, WorkspaceLimits, WorkspaceManager};
use crate::harness_executor::{execute_harness_assignment, HarnessAssignment, JsonHarnessModel};
use crate::harness_loop::{
    HarnessLoopObserver, LoopBudgets, LoopFailure, ModelAction, ToolRequest, ToolResult,
};
use crate::harness_tools::{
    NetworkPolicy, ValidationIsolation, ValidationProfile, ValidationRunner,
};
use crate::http::{signed_runner_get_json, signed_runner_post_json_body};
use crate::identity::DeviceIdentity;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Serialize)]
pub struct HarnessCapabilityAdvertisement {
    pub execution_modes: Vec<String>,
    pub supported_operations: Vec<String>,
    pub sandbox_runtime: Option<String>,
    pub network_default_disabled: bool,
    pub max_workspace_mb: u32,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    mode: &'static str,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HarnessTaskContract {
    pub task_id: String,
    pub repository_source_id: String,
    pub objective: String,
    pub base_revision: String,
    pub allowed_path_prefixes: Vec<String>,
    pub execution_mode: String,
    pub allowed_operations: Vec<String>,
    pub validation_profiles: Vec<String>,
    pub budgets: HarnessBudgetsContract,
    pub state: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HarnessBudgetsContract {
    pub max_wall_time_ms: u64,
    pub max_cpu_time_ms: u64,
    pub max_memory_mb: u32,
    pub max_disk_mb: u32,
    pub max_output_bytes: u64,
    pub max_tool_calls: u32,
    pub max_model_turns: u32,
    pub max_repair_attempts: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HarnessAttemptContract {
    pub attempt_id: String,
    pub task_id: String,
    #[serde(alias = "node_id")]
    pub runner_id: String,
    pub state: String,
    pub state_version: u64,
}

#[derive(Serialize)]
struct HarnessRunnerRegistrationRequest {
    runner_id: String,
    device_id: String,
    public_key_hex: String,
    kind: &'static str,
    owner_user_id: Option<String>,
    pairing_code: Option<String>,
    tenant_ids: Vec<String>,
    repository_source_ids: Vec<String>,
    local_projects: Vec<String>,
    execution_modes: Vec<String>,
    supported_operations: Vec<String>,
    network_default_disabled: bool,
    max_workspace_mb: u32,
    usable_memory_mb: u32,
    parallel_slots: u32,
    trusted_identity: bool,
    ready: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HarnessRunnerRegistrationResponse {
    pub runner_id: String,
    pub owner_user_id: Option<String>,
    pub tenant_ids: Vec<String>,
    pub repository_source_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HarnessClaimResponse {
    pub task: Option<HarnessTaskContract>,
    pub attempt: Option<HarnessAttemptContract>,
}

#[derive(Serialize)]
struct TransitionRequest<'a> {
    expected_state_version: u64,
    state: &'a str,
    workspace_id: Option<&'a str>,
    code: Option<&'a str>,
}

pub fn capabilities(config: &RunnerConfig) -> Result<HarnessCapabilityAdvertisement, String> {
    let workspace_root = required_absolute(&config.workspace_root, "workspace root")?;
    let projects_root = required_absolute(&config.projects_root, "local projects root")?;
    let git = required_absolute(&config.git_executable, "Git executable")?;
    if (config.repositories.is_empty() && config.repository_source_patterns.is_empty())
        || config.tenant_ids.is_empty()
        || config.validation_profiles.is_empty()
        || !workspace_root.is_absolute()
        || !projects_root.is_absolute()
        || !git.is_absolute()
        || config
            .validation_profiles
            .values()
            .any(|profile| !PathBuf::from(&profile.executable).is_absolute())
    {
        return Err(
            "HARNESS_RUNNER_SCOPE_REQUIRED: repositories, tenants, and absolute validation configuration are required"
                .to_string(),
        );
    }
    let sandbox_runtime = config
        .sandbox_runtime
        .as_ref()
        .map(PathBuf::from)
        .filter(|runtime| runtime.is_absolute())
        .filter(|_| {
            config
                .sandbox_image_digest
                .as_deref()
                .is_some_and(|image| image.contains("@sha256:"))
        })
        .and_then(|runtime| {
            std::process::Command::new(&runtime)
                .arg("--version")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|_| runtime)
        });
    let mut execution_modes = vec!["hybrid".to_string()];
    if sandbox_runtime.is_some() {
        execution_modes.insert(0, "sandbox".to_string());
    }
    Ok(HarnessCapabilityAdvertisement {
        execution_modes,
        supported_operations: vec![
            "repository.status".to_string(),
            "repository.diff".to_string(),
            "file.read".to_string(),
            "file.search".to_string(),
            "patch.apply".to_string(),
            "validation.run".to_string(),
            "artifact.publish".to_string(),
        ],
        sandbox_runtime: sandbox_runtime.map(|runtime| runtime.display().to_string()),
        network_default_disabled: true,
        max_workspace_mb: config.max_workspace_mb.max(1),
    })
}

pub fn register_runner(
    config: &RunnerConfig,
    identity: &DeviceIdentity,
    capabilities: &HarnessCapabilityAdvertisement,
    usable_memory_mb: u32,
    trusted_identity: bool,
    pairing_code: Option<&str>,
) -> Result<HarnessRunnerRegistrationResponse, String> {
    let runner_id = required_runner_id(config)?;
    let owner_user_id = config.owner_user_id.trim();
    if owner_user_id.is_empty() && pairing_code.is_none() {
        return Err(
            "HARNESS_RUNNER_OWNER_REQUIRED: local runner must be paired to one user".to_string(),
        );
    }
    if config.tenant_ids.is_empty()
        || (config.repositories.is_empty() && config.repository_source_patterns.is_empty())
    {
        return Err(
            "HARNESS_RUNNER_SCOPE_REQUIRED: tenant and repository scopes are required".to_string(),
        );
    }
    let request = HarnessRunnerRegistrationRequest {
        runner_id: runner_id.to_string(),
        device_id: config.device_id.clone(),
        public_key_hex: identity.public_key_hex.clone(),
        kind: "local_user",
        owner_user_id: (!owner_user_id.is_empty()).then(|| owner_user_id.to_string()),
        pairing_code: pairing_code.map(str::to_string),
        tenant_ids: config.tenant_ids.clone(),
        repository_source_ids: config
            .repositories
            .keys()
            .cloned()
            .chain(config.repository_source_patterns.iter().cloned())
            .collect(),
        local_projects: local_project_slugs(config),
        execution_modes: capabilities.execution_modes.clone(),
        supported_operations: capabilities.supported_operations.clone(),
        network_default_disabled: capabilities.network_default_disabled,
        max_workspace_mb: capabilities.max_workspace_mb,
        usable_memory_mb: usable_memory_mb.max(1),
        parallel_slots: config.parallel_slots.clamp(1, 64),
        trusted_identity,
        ready: true,
    };
    signed_runner_post_json_body(
        &config.control_plane_url,
        "/internal/harness/runners/register",
        runner_id,
        identity,
        &request,
    )
}

fn valid_local_project_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

pub fn local_project_slugs(config: &RunnerConfig) -> Vec<String> {
    let Some(root) = config.projects_root.as_deref().map(PathBuf::from) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut projects = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
        })
        .filter_map(|entry| {
            let slug = entry.file_name().to_str()?.to_string();
            if !valid_local_project_slug(&slug) {
                return None;
            }
            let metadata_path = entry.path().join(".mundusx/project.json");
            let metadata = fs::metadata(&metadata_path).ok()?;
            if !metadata.is_file() || metadata.len() > 65_536 {
                return None;
            }
            let document: serde_json::Value =
                serde_json::from_slice(&fs::read(metadata_path).ok()?).ok()?;
            (document.get("name")?.as_str()? == slug).then_some(slug)
        })
        .collect::<Vec<_>>();
    projects.sort();
    projects.dedup();
    projects.truncate(100);
    projects
}

fn required_runner_id(config: &RunnerConfig) -> Result<&str, String> {
    (!config.runner_id.trim().is_empty())
        .then_some(config.runner_id.as_str())
        .ok_or_else(|| "HARNESS_RUNNER_UNAVAILABLE: local runner is not configured".to_string())
}

pub fn claim_next(
    config: &RunnerConfig,
    identity: &DeviceIdentity,
) -> Result<Option<(HarnessTaskContract, HarnessAttemptContract)>, String> {
    let runner_id = required_runner_id(config)?;
    let path = format!("/internal/harness/runners/attempts/next?runner_id={runner_id}");
    let response = signed_runner_get_json::<HarnessClaimResponse>(
        &config.control_plane_url,
        &path,
        runner_id,
        identity,
    )?;
    match (response.task, response.attempt) {
        (Some(task), Some(attempt)) => Ok(Some((task, attempt))),
        (None, None) => Ok(None),
        _ => Err("HARNESS_RESPONSE_INVALID: incomplete task assignment".to_string()),
    }
}

pub fn execute_claim(
    config: &RunnerConfig,
    identity: &DeviceIdentity,
    task: HarnessTaskContract,
    mut attempt: HarnessAttemptContract,
) -> Result<(), String> {
    if config.runner_id != attempt.runner_id || attempt.task_id != task.task_id {
        return Err("HARNESS_AUTH_REQUIRED: assignment owner mismatch".to_string());
    }
    if task.state == "cancelling" || task.state == "cancelled" {
        transition(
            config,
            identity,
            &attempt,
            "cancelled",
            None,
            Some("HARNESS_CANCELLED"),
        )?;
        return Err("HARNESS_CANCELLED: assignment was cancelled before execution".to_string());
    }
    let source =
        resolve_repository_source(config, &task.repository_source_id, &task.base_revision)?;
    let workspace_root = required_absolute(&config.workspace_root, "workspace root")?;
    let git = required_absolute(&config.git_executable, "Git executable")?;
    let manager = WorkspaceManager::new(workspace_root, git).map_err(display_harness)?;
    let (runner, profiles) = validation_config(config, &task.execution_mode)?;

    let workspace_id = format!("ws_{}", attempt.attempt_id.trim_start_matches("hattempt_"));
    attempt = transition(
        config,
        identity,
        &attempt,
        "preparing",
        Some(&workspace_id),
        None,
    )?;
    attempt = transition(config, identity, &attempt, "running", None, None)?;

    let version = std::sync::Mutex::new(attempt.state_version);
    let mut observer = ControlPlaneObserver {
        config,
        identity,
        attempt_id: &attempt.attempt_id,
        version: &version,
    };
    let objective = task.objective.clone();
    let mut model = JsonHarnessModel::new(
        |system_prompt: &str, prompt: &str| {
            let request = ChatCompletionRequest {
                model: &config.inference_model,
                stream: false,
                messages: vec![
                    ChatMessage {
                        role: "system",
                        content: system_prompt,
                    },
                    ChatMessage {
                        role: "user",
                        content: prompt,
                    },
                ],
                max_tokens: 4_096,
                temperature: 0.1,
                top_p: 0.9,
                mode: "single",
            };
            let response: ChatCompletionResponse = signed_runner_post_json_body(
                &config.control_plane_url,
                "/v1/chat/completions",
                &config.runner_id,
                identity,
                &request,
            )?;
            response
                .choices
                .into_iter()
                .next()
                .map(|choice| choice.message.content)
                .filter(|content| !content.trim().is_empty())
                .ok_or_else(|| {
                    "HARNESS_MODEL_RESPONSE_INVALID: empty inference response".to_string()
                })
        },
        objective,
    );
    let cancelled = AtomicBool::new(false);
    let assignment = HarnessAssignment {
        task_id: task.task_id,
        attempt_id: attempt.attempt_id.clone(),
        repository_source_id: task.repository_source_id,
        objective: task.objective,
        source_repository: source.path,
        base_revision: source.revision,
        persist_to_source: source.persist_changes,
        allowed_path_prefixes: task
            .allowed_path_prefixes
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        allowed_operations: task.allowed_operations.into_iter().collect(),
        validation_profiles: task.validation_profiles.into_iter().collect(),
        workspace_limits: WorkspaceLimits {
            max_disk_mb: task.budgets.max_disk_mb,
            max_memory_mb: task.budgets.max_memory_mb,
            max_cpu_time_ms: task.budgets.max_cpu_time_ms,
            max_wall_time_ms: task.budgets.max_wall_time_ms,
        },
        loop_budgets: LoopBudgets {
            max_wall_time_ms: task.budgets.max_wall_time_ms,
            max_model_turns: task.budgets.max_model_turns,
            max_tool_calls: task.budgets.max_tool_calls,
            max_repair_attempts: task.budgets.max_repair_attempts,
            max_output_bytes: task.budgets.max_output_bytes,
        },
    };
    let report = match execute_harness_assignment(
        &manager,
        &runner,
        &profiles,
        assignment,
        &mut model,
        &mut observer,
        &cancelled,
        || false,
    ) {
        Ok(report) => report,
        Err(error) => {
            attempt.state_version = *version
                .lock()
                .map_err(|_| "harness version lock poisoned")?;
            let _ = transition(
                config,
                identity,
                &attempt,
                "failed",
                None,
                Some(&error.code),
            );
            return Err(display_harness(error));
        }
    };

    attempt.state_version = *version
        .lock()
        .map_err(|_| "harness version lock poisoned")?;
    attempt = transition(config, identity, &attempt, "validating", None, None)?;
    if report.outcome.status == "completed" {
        if let Some(digest) = report.patch_sha256.as_deref() {
            let payload = serde_json::json!({
                "expected_attempt_state_version": attempt.state_version,
                "kind": "patch",
                "sha256": digest,
                "size_bytes": report.patch_size_bytes,
                "storage_reference": null,
                "base_revision": task.base_revision,
                "changed_paths": report.changed_paths,
                "verification_level": "partially_verified",
            });
            post_evidence(config, identity, &attempt.attempt_id, "artifacts", &payload)?;
            attempt.state_version += 1;
        }
        transition(config, identity, &attempt, "succeeded", None, None)?;
        Ok(())
    } else {
        let code = report
            .outcome
            .code
            .as_deref()
            .unwrap_or("HARNESS_EXECUTION_FAILED");
        transition(config, identity, &attempt, "failed", None, Some(code))?;
        Err(format!("{code}: harness attempt did not complete"))
    }
}

struct ControlPlaneObserver<'a> {
    config: &'a RunnerConfig,
    identity: &'a DeviceIdentity,
    attempt_id: &'a str,
    version: &'a std::sync::Mutex<u64>,
}

impl HarnessLoopObserver for ControlPlaneObserver<'_> {
    fn model_turn(&mut self, action: &ModelAction) -> Result<(), LoopFailure> {
        let runner_id = required_runner_id(self.config).map_err(control_loop_failure)?;
        let path = format!("/internal/harness/runners/attempts/next?runner_id={runner_id}");
        let current = signed_runner_get_json::<HarnessClaimResponse>(
            &self.config.control_plane_url,
            &path,
            runner_id,
            self.identity,
        )
        .map_err(control_loop_failure)?;
        if current
            .task
            .as_ref()
            .is_some_and(|task| task.state == "cancelling" || task.state == "cancelled")
        {
            return Err(LoopFailure::new(
                "HARNESS_CANCELLED",
                "control plane cancelled the task",
            ));
        }
        let bytes = serde_json::to_vec(action).map_err(json_loop_failure)?;
        let mut version = self.version.lock().map_err(|_| {
            LoopFailure::new("HARNESS_STATE_CONFLICT", "attempt version lock poisoned")
        })?;
        let payload = serde_json::json!({
            "expected_attempt_state_version": *version,
            "progress_sha256": format!("{:x}", Sha256::digest(&bytes)),
            "output_bytes": bytes.len(),
            "repair_attempt": false,
        });
        post_evidence(
            self.config,
            self.identity,
            self.attempt_id,
            "model-turns",
            &payload,
        )
        .map_err(control_loop_failure)?;
        *version += 1;
        Ok(())
    }

    fn tool_call(&mut self, request: &ToolRequest, result: &ToolResult) -> Result<(), LoopFailure> {
        let input = serde_json::to_vec(&request.input).map_err(json_loop_failure)?;
        let output = serde_json::to_vec(&result.output).map_err(json_loop_failure)?;
        let mut version = self.version.lock().map_err(|_| {
            LoopFailure::new("HARNESS_STATE_CONFLICT", "attempt version lock poisoned")
        })?;
        let payload = serde_json::json!({
            "expected_attempt_state_version": *version,
            "tool_call_id": request.tool_call_id,
            "operation": request.operation,
            "idempotency_key": request.idempotency_key,
            "input_sha256": format!("{:x}", Sha256::digest(&input)),
            "output_sha256": format!("{:x}", Sha256::digest(&output)),
            "status": result.status,
            "code": result.code,
            "duration_ms": null,
            "output_bytes": result.output_bytes,
        });
        post_evidence(
            self.config,
            self.identity,
            self.attempt_id,
            "tool-calls",
            &payload,
        )
        .map_err(control_loop_failure)?;
        *version += 1;
        if request.operation == "validation.run" {
            let validation = serde_json::json!({
                "expected_attempt_state_version": *version,
                "profile_id": result.output.get("profile_id").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "profile_version": "node-config-v1",
                "status": if result.status == "succeeded" { "passed" } else { "failed" },
                "code": result.code,
                "exit_code": result.output.get("exit_code"),
                "duration_ms": result.output.get("duration_ms"),
                "output_sha256": result.output.get("output_sha256"),
                "artifact_sha256": result.output.get("artifact_sha256"),
                "base_revision": result.output.get("base_revision"),
                "environment_sha256": result.output.get("environment_sha256"),
                "output_truncated": result.output.get("output_truncated").and_then(|v| v.as_bool()).unwrap_or(false),
            });
            post_evidence(
                self.config,
                self.identity,
                self.attempt_id,
                "validations",
                &validation,
            )
            .map_err(control_loop_failure)?;
            *version += 1;
        }
        Ok(())
    }
}

fn transition(
    config: &RunnerConfig,
    identity: &DeviceIdentity,
    attempt: &HarnessAttemptContract,
    state: &str,
    workspace_id: Option<&str>,
    code: Option<&str>,
) -> Result<HarnessAttemptContract, String> {
    let path = format!(
        "/internal/harness/runners/attempts/{}/transition",
        attempt.attempt_id
    );
    signed_runner_post_json_body(
        &config.control_plane_url,
        &path,
        required_runner_id(config)?,
        identity,
        &TransitionRequest {
            expected_state_version: attempt.state_version,
            state,
            workspace_id,
            code,
        },
    )
}

fn post_evidence(
    config: &RunnerConfig,
    identity: &DeviceIdentity,
    attempt_id: &str,
    action: &str,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let path = format!("/internal/harness/runners/attempts/{attempt_id}/{action}");
    let _: serde_json::Value = signed_runner_post_json_body(
        &config.control_plane_url,
        &path,
        required_runner_id(config)?,
        identity,
        payload,
    )?;
    Ok(())
}

fn validation_config(
    config: &RunnerConfig,
    execution_mode: &str,
) -> Result<(ValidationRunner, BTreeMap<String, ValidationProfile>), String> {
    let isolation = if execution_mode == "sandbox" {
        ValidationIsolation::DockerSandbox {
            runtime: required_absolute(&config.sandbox_runtime, "sandbox runtime")?,
            image_digest: config
                .sandbox_image_digest
                .clone()
                .ok_or_else(|| "HARNESS_POLICY_DENIED: sandbox image digest missing".to_string())?,
        }
    } else {
        ValidationIsolation::TrustedHybrid
    };
    let runner = ValidationRunner::new(isolation).map_err(display_harness)?;
    let profiles = config
        .validation_profiles
        .iter()
        .map(|(id, value)| {
            Ok((
                id.clone(),
                ValidationProfile {
                    profile_id: id.clone(),
                    executable: PathBuf::from(&value.executable),
                    arguments: value.arguments.clone(),
                    working_directory: PathBuf::from(&value.working_directory),
                    environment: value.environment.clone(),
                    network: if value.network_allowed {
                        NetworkPolicy::Allowed
                    } else {
                        NetworkPolicy::Disabled
                    },
                    timeout_ms: value.timeout_ms,
                    max_output_bytes: value.max_output_bytes,
                    max_memory_mb: value.max_memory_mb,
                    max_cpu_time_ms: value.max_cpu_time_ms,
                    max_processes: value.max_processes,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    Ok((runner, profiles))
}

fn required_absolute(value: &Option<String>, label: &str) -> Result<PathBuf, String> {
    let path = value
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| format!("HARNESS_POLICY_DENIED: {label} is not configured"))?;
    if !path.is_absolute() {
        return Err(format!("HARNESS_POLICY_DENIED: {label} must be absolute"));
    }
    Ok(path)
}

struct ResolvedRepositorySource {
    path: PathBuf,
    revision: String,
    persist_changes: bool,
}

fn resolve_repository_source(
    config: &RunnerConfig,
    repository_source_id: &str,
    base_revision: &str,
) -> Result<ResolvedRepositorySource, String> {
    if let Some(path) = config.repositories.get(repository_source_id) {
        return Ok(ResolvedRepositorySource {
            path: PathBuf::from(path),
            revision: base_revision.to_string(),
            persist_changes: false,
        });
    }
    if !config
        .repository_source_patterns
        .iter()
        .any(|prefix| prefix.ends_with(':') && repository_source_id.starts_with(prefix))
    {
        return Err("HARNESS_REPOSITORY_SOURCE_DENIED: unknown source id".to_string());
    }
    if repository_source_id.starts_with("local-project:") {
        return resolve_local_project(config, repository_source_id, base_revision);
    }
    let (repository_id, full_name) = parse_github_source(repository_source_id)?;
    if base_revision.len() != 40 || !base_revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "HARNESS_BASE_REVISION_INVALID: GitHub revision must be a commit SHA".to_string(),
        );
    }
    let github_cli = required_absolute(&config.github_cli, "GitHub CLI")?;
    let git = required_absolute(&config.git_executable, "Git executable")?;
    let cache_root = crate::config::config_dir().join("repositories");
    fs::create_dir_all(&cache_root)
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    let source = cache_root.join(format!(
        "{}-{}.git",
        repository_id,
        base_revision.to_ascii_lowercase()
    ));
    if !source.exists() {
        let partial = cache_root.join(format!(
            ".{}-{}.partial",
            repository_id,
            uuid::Uuid::new_v4().simple()
        ));
        let output = Command::new(&github_cli)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(["repo", "clone", full_name])
            .arg(&partial)
            .args(["--", "--bare", "--filter=blob:none"])
            .output()
            .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
        if !output.status.success() {
            let _ = remove_partial_repository(&cache_root, &partial);
            return Err(format!(
                "HARNESS_GITHUB_AUTH_REQUIRED: GitHub CLI could not clone {full_name}: {}",
                bounded_stderr(&output.stderr)
            ));
        }
        fs::rename(&partial, &source).map_err(|error| {
            let _ = remove_partial_repository(&cache_root, &partial);
            format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}")
        })?;
    }
    let object = format!("{}^{{commit}}", base_revision.to_ascii_lowercase());
    let verified = Command::new(git)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(["--git-dir"])
        .arg(&source)
        .args(["cat-file", "-e", &object])
        .status()
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    if !verified.success() {
        return Err(
            "HARNESS_BASE_REVISION_INVALID: authorized repository does not contain the pinned commit"
                .to_string(),
        );
    }
    Ok(ResolvedRepositorySource {
        path: source,
        revision: base_revision.to_ascii_lowercase(),
        persist_changes: false,
    })
}

fn resolve_local_project(
    config: &RunnerConfig,
    repository_source_id: &str,
    base_revision: &str,
) -> Result<ResolvedRepositorySource, String> {
    if base_revision != "0000000000000000000000000000000000000000" {
        return Err(
            "HARNESS_BASE_REVISION_INVALID: local project bootstrap revision is invalid"
                .to_string(),
        );
    }
    let mut parts = repository_source_id.splitn(4, ':');
    if parts.next() != Some("local-project") {
        return Err("HARNESS_REPOSITORY_SOURCE_DENIED: malformed local project source".to_string());
    }
    let owner = parts.next().unwrap_or_default();
    let template = parts.next().unwrap_or_default();
    let slug = parts.next().unwrap_or_default();
    if owner != config.owner_user_id
        || !matches!(template, "generic" | "java-maven")
        || !valid_local_project_slug(slug)
    {
        return Err(
            "HARNESS_REPOSITORY_SOURCE_DENIED: local project owner, template, or slug is invalid"
                .to_string(),
        );
    }
    let root = required_absolute(&config.projects_root, "local projects root")?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    let root = fs::canonicalize(&root)
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    let project = root.join(slug);
    if project.parent() != Some(root.as_path()) {
        return Err(
            "HARNESS_PATH_DENIED: local project must be a direct child of the projects root"
                .to_string(),
        );
    }
    if !project.exists() {
        fs::create_dir(&project)
            .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
        scaffold_local_project(&project, slug, template)?;
        initialize_local_repository(config, &project)?;
    }
    let project = fs::canonicalize(&project)
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    if project.parent() != Some(root.as_path()) || !project.is_dir() {
        return Err(
            "HARNESS_PATH_DENIED: local project resolved outside the projects root".to_string(),
        );
    }
    let revision = git_stdout(config, &project, &["rev-parse", "HEAD"])?;
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "HARNESS_BASE_REVISION_INVALID: local project has no valid Git revision".to_string(),
        );
    }
    Ok(ResolvedRepositorySource {
        path: project,
        revision: revision.to_ascii_lowercase(),
        persist_changes: true,
    })
}

fn scaffold_local_project(project: &Path, slug: &str, template: &str) -> Result<(), String> {
    fs::create_dir_all(project.join(".mundusx"))
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    let metadata = serde_json::to_vec_pretty(&serde_json::json!({
        "version": 1,
        "name": slug,
        "template": template
    }))
    .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    fs::write(project.join(".mundusx/project.json"), metadata)
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    fs::write(
        project.join("README.md"),
        format!("# {slug}\n\nCreated locally by MundusX.\n"),
    )
    .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    if template == "java-maven" {
        fs::create_dir_all(project.join("src/main/java"))
            .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
        fs::create_dir_all(project.join("src/test/java"))
            .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
        fs::write(project.join("pom.xml"), "<project xmlns=\"http://maven.apache.org/POM/4.0.0\"><modelVersion>4.0.0</modelVersion><groupId>local.mundusx</groupId><artifactId>project</artifactId><version>0.1.0</version><properties><maven.compiler.release>21</maven.compiler.release></properties></project>\n")
            .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    }
    Ok(())
}

fn initialize_local_repository(config: &RunnerConfig, project: &Path) -> Result<(), String> {
    git_stdout(config, project, &["init"])?;
    git_stdout(config, project, &["add", "--all"])?;
    git_stdout(
        config,
        project,
        &[
            "-c",
            "user.name=MundusX Harness",
            "-c",
            "user.email=harness@localhost",
            "commit",
            "-m",
            "mundusx: initialize local project",
        ],
    )?;
    Ok(())
}

fn git_stdout(
    config: &RunnerConfig,
    directory: &Path,
    arguments: &[&str],
) -> Result<String, String> {
    let git = required_absolute(&config.git_executable, "Git executable")?;
    let output = Command::new(git)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(directory)
        .args(arguments)
        .output()
        .map_err(|error| format!("HARNESS_REPOSITORY_PREPARE_FAILED: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "HARNESS_REPOSITORY_PREPARE_FAILED: {}",
            bounded_stderr(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_github_source(value: &str) -> Result<(&str, &str), String> {
    let mut parts = value.splitn(3, ':');
    if parts.next() != Some("github") {
        return Err(
            "HARNESS_REPOSITORY_SOURCE_DENIED: unsupported repository provider".to_string(),
        );
    }
    let repository_id = parts.next().unwrap_or_default();
    let full_name = parts.next().unwrap_or_default();
    let valid_name = full_name.split('/').count() == 2
        && full_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'));
    if repository_id.is_empty()
        || !repository_id.bytes().all(|byte| byte.is_ascii_digit())
        || !valid_name
    {
        return Err("HARNESS_REPOSITORY_SOURCE_DENIED: malformed GitHub source id".to_string());
    }
    Ok((repository_id, full_name))
}

fn remove_partial_repository(root: &Path, partial: &Path) -> std::io::Result<()> {
    if partial.parent() != Some(root) || !partial.starts_with(root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "partial repository escaped cache root",
        ));
    }
    if partial.exists() {
        fs::remove_dir_all(partial)?;
    }
    Ok(())
}

fn bounded_stderr(stderr: &[u8]) -> String {
    String::from_utf8_lossy(&stderr[..stderr.len().min(2_048)])
        .trim()
        .to_string()
}

fn display_harness(error: HarnessError) -> String {
    error.to_string()
}

fn json_loop_failure(error: serde_json::Error) -> LoopFailure {
    LoopFailure::new("HARNESS_EVIDENCE_INVALID", error.to_string())
}

fn control_loop_failure(error: String) -> LoopFailure {
    LoopFailure::new("HARNESS_EVIDENCE_PERSIST_FAILED", error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HarnessValidationProfileConfig;

    #[test]
    fn github_source_ids_are_strict_and_provider_scoped() {
        assert_eq!(
            parse_github_source("github:42:owner/repo").unwrap(),
            ("42", "owner/repo")
        );
        assert!(parse_github_source("github:42:owner/repo/extra").is_err());
        assert!(parse_github_source("github:not-a-number:owner/repo").is_err());
        assert!(parse_github_source("gitlab:42:owner/repo").is_err());
    }

    #[test]
    fn harness_configuration_requires_absolute_operator_owned_paths() {
        assert!(required_absolute(&Some("relative/path".to_string()), "root").is_err());
        let absolute = if cfg!(windows) {
            "C:\\harness\\work"
        } else {
            "/harness/work"
        };
        assert_eq!(
            required_absolute(&Some(absolute.to_string()), "root").unwrap(),
            PathBuf::from(absolute)
        );
    }

    #[test]
    fn local_project_source_is_owner_bound_lowercase_and_git_backed() {
        let mut config = RunnerConfig::default();
        let root = std::env::temp_dir().join(format!(
            "mundusx-local-projects-{}",
            uuid::Uuid::new_v4().simple()
        ));
        config.projects_root = Some(root.display().to_string());
        config.owner_user_id = "78a1c06a-861c-43b4-b7db-b54a51fc912d".to_string();
        let source = resolve_local_project(
            &config,
            "local-project:78a1c06a-861c-43b4-b7db-b54a51fc912d:java-maven:hello-java",
            "0000000000000000000000000000000000000000",
        )
        .expect("create local project");
        assert_eq!(
            source.path,
            fs::canonicalize(root.join("hello-java")).expect("canonical project path")
        );
        assert!(source.persist_changes);
        assert_eq!(source.revision.len(), 40);
        assert!(source.path.join("pom.xml").is_file());
        assert_eq!(local_project_slugs(&config), vec!["hello-java"]);
        assert!(resolve_local_project(
            &config,
            "local-project:another-user:generic:hello-java",
            "0000000000000000000000000000000000000000",
        )
        .is_err());
        assert!(resolve_local_project(
            &config,
            "local-project:78a1c06a-861c-43b4-b7db-b54a51fc912d:generic:Hello Java",
            "0000000000000000000000000000000000000000",
        )
        .is_err());
        fs::remove_dir_all(root).expect("cleanup local project fixture");
    }

    #[test]
    fn local_project_inventory_ignores_unmanaged_invalid_and_nested_directories() {
        let mut config = RunnerConfig::default();
        let root = std::env::temp_dir().join(format!(
            "mundusx-project-inventory-{}",
            uuid::Uuid::new_v4().simple()
        ));
        config.projects_root = Some(root.display().to_string());
        fs::create_dir_all(root.join("alpha/.mundusx")).expect("alpha metadata directory");
        fs::write(
            root.join("alpha/.mundusx/project.json"),
            br#"{"version":1,"name":"alpha"}"#,
        )
        .expect("alpha metadata");
        fs::create_dir_all(root.join("unmanaged")).expect("unmanaged directory");
        fs::create_dir_all(root.join("Bad Name/.mundusx")).expect("invalid directory");
        fs::write(
            root.join("Bad Name/.mundusx/project.json"),
            br#"{"version":1,"name":"Bad Name"}"#,
        )
        .expect("invalid metadata");
        fs::create_dir_all(root.join("wrong-name/.mundusx")).expect("mismatch directory");
        fs::write(
            root.join("wrong-name/.mundusx/project.json"),
            br#"{"version":1,"name":"another"}"#,
        )
        .expect("mismatch metadata");
        fs::create_dir_all(root.join("alpha/nested/.mundusx")).expect("nested metadata directory");
        fs::write(
            root.join("alpha/nested/.mundusx/project.json"),
            br#"{"version":1,"name":"nested"}"#,
        )
        .expect("nested metadata");

        assert_eq!(local_project_slugs(&config), vec!["alpha"]);
        fs::remove_dir_all(root).expect("cleanup inventory fixture");
    }

    #[test]
    fn sandbox_requires_digest_pinned_operator_configuration() {
        let mut config = RunnerConfig::default();
        config.sandbox_runtime = Some(if cfg!(windows) {
            "C:\\Program Files\\Docker\\docker.exe".to_string()
        } else {
            "/usr/bin/docker".to_string()
        });
        config.sandbox_image_digest = Some("mutable:latest".to_string());
        assert!(validation_config(&config, "sandbox").is_err());

        config.sandbox_image_digest = Some(
            "harness@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        );
        assert!(validation_config(&config, "sandbox").is_ok());
    }

    #[test]
    fn validation_profiles_are_built_only_from_node_configuration() {
        let mut config = RunnerConfig::default();
        config.validation_profiles.insert(
            "rust-default".to_string(),
            HarnessValidationProfileConfig {
                executable: if cfg!(windows) {
                    "C:\\tools\\cargo.exe"
                } else {
                    "/usr/bin/cargo"
                }
                .to_string(),
                arguments: vec!["test".to_string()],
                working_directory: "src".to_string(),
                environment: BTreeMap::new(),
                network_allowed: false,
                timeout_ms: 30_000,
                max_output_bytes: 16_384,
                max_memory_mb: 1_024,
                max_cpu_time_ms: 30_000,
                max_processes: 32,
            },
        );
        let (_, profiles) = validation_config(&config, "hybrid").unwrap();
        assert_eq!(profiles["rust-default"].arguments, ["test"]);
        assert_eq!(profiles["rust-default"].network, NetworkPolicy::Disabled);
    }
}
