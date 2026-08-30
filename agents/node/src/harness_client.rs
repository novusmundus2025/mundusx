//! Signed control-plane client and concrete node runner for Coding Harness v1.

use crate::contracts::{Backend, HarnessCapabilityAdvertisement, WorkerLaunchRequest};
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
use crate::storage::AgentConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

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
    owner_user_id: String,
    tenant_ids: Vec<String>,
    repository_source_ids: Vec<String>,
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

pub fn register_runner(
    config: &AgentConfig,
    identity: &DeviceIdentity,
    capabilities: &HarnessCapabilityAdvertisement,
    usable_memory_mb: u32,
    trusted_identity: bool,
) -> Result<(), String> {
    let runner_id = required_runner_id(config)?;
    let owner_user_id = config
        .harness_runner_owner_user_id
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "HARNESS_RUNNER_OWNER_REQUIRED: local runner must be paired to one user".to_string()
        })?;
    if config.harness_runner_tenant_ids.is_empty() || config.harness_repositories.is_empty() {
        return Err(
            "HARNESS_RUNNER_SCOPE_REQUIRED: tenant and repository scopes are required".to_string(),
        );
    }
    let request = HarnessRunnerRegistrationRequest {
        runner_id: runner_id.to_string(),
        device_id: config.device_id.clone(),
        public_key_hex: identity.public_key_hex.clone(),
        kind: "local_user",
        owner_user_id: owner_user_id.clone(),
        tenant_ids: config.harness_runner_tenant_ids.clone(),
        repository_source_ids: config.harness_repositories.keys().cloned().collect(),
        execution_modes: capabilities.execution_modes.clone(),
        supported_operations: capabilities.supported_operations.clone(),
        network_default_disabled: capabilities.network_default_disabled,
        max_workspace_mb: capabilities.max_workspace_mb,
        usable_memory_mb: usable_memory_mb.max(1),
        parallel_slots: config.harness_runner_slots.clamp(1, 64),
        trusted_identity,
        ready: true,
    };
    let _: serde_json::Value = signed_runner_post_json_body(
        &config.control_plane_url,
        "/internal/harness/runners/register",
        runner_id,
        identity,
        &request,
    )?;
    Ok(())
}

fn required_runner_id(config: &AgentConfig) -> Result<&str, String> {
    config
        .harness_runner_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "HARNESS_RUNNER_UNAVAILABLE: local runner is not configured".to_string())
}

pub fn claim_next(
    config: &AgentConfig,
    identity: &DeviceIdentity,
) -> Result<Option<(HarnessTaskContract, HarnessAttemptContract)>, String> {
    let Some(runner_id) = config.harness_runner_id.as_deref() else {
        return Ok(None);
    };
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
    config: &AgentConfig,
    identity: &DeviceIdentity,
    task: HarnessTaskContract,
    mut attempt: HarnessAttemptContract,
) -> Result<(), String> {
    if config.harness_runner_id.as_deref() != Some(&attempt.runner_id)
        || attempt.task_id != task.task_id
    {
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
    let source = config
        .harness_repositories
        .get(&task.repository_source_id)
        .map(PathBuf::from)
        .ok_or_else(|| "HARNESS_REPOSITORY_SOURCE_DENIED: unknown source id".to_string())?;
    let workspace_root = required_absolute(&config.harness_workspace_root, "workspace root")?;
    let git = required_absolute(&config.harness_git_executable, "Git executable")?;
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
            let request = WorkerLaunchRequest {
                job_id: format!("harness-{}", attempt.attempt_id),
                node_id: config.device_id.clone(),
                backend: resolved_backend(config.backend_preference),
                stream: false,
                prompt: prompt.to_string(),
                model: config.active_model.clone(),
                mode: Some("harness".to_string()),
                system_prompt: Some(system_prompt.to_string()),
                max_tokens: Some(4096),
                temperature: Some(0.1),
                top_p: Some(0.9),
                seed: None,
            };
            let response = crate::worker::launch_worker(&request, &config.effective_model_dir())?;
            if response.status == "completed" {
                Ok(response.output)
            } else {
                Err(response
                    .error
                    .unwrap_or_else(|| "model worker failed".to_string()))
            }
        },
        objective,
    );
    let cancelled = AtomicBool::new(false);
    let assignment = HarnessAssignment {
        task_id: task.task_id,
        attempt_id: attempt.attempt_id.clone(),
        repository_source_id: task.repository_source_id,
        objective: task.objective,
        source_repository: source,
        base_revision: task.base_revision.clone(),
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
    config: &'a AgentConfig,
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
    config: &AgentConfig,
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
    config: &AgentConfig,
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
    config: &AgentConfig,
    execution_mode: &str,
) -> Result<(ValidationRunner, BTreeMap<String, ValidationProfile>), String> {
    let isolation = if execution_mode == "sandbox" {
        ValidationIsolation::DockerSandbox {
            runtime: required_absolute(&config.harness_sandbox_runtime, "sandbox runtime")?,
            image_digest: config
                .harness_sandbox_image_digest
                .clone()
                .ok_or_else(|| "HARNESS_POLICY_DENIED: sandbox image digest missing".to_string())?,
        }
    } else {
        ValidationIsolation::TrustedHybrid
    };
    let runner = ValidationRunner::new(isolation).map_err(display_harness)?;
    let profiles = config
        .harness_validation_profiles
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

fn resolved_backend(value: Backend) -> Backend {
    value
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
    use crate::storage::HarnessValidationProfileConfig;

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
    fn sandbox_requires_digest_pinned_operator_configuration() {
        let mut config = AgentConfig::default();
        config.harness_sandbox_runtime = Some(if cfg!(windows) {
            "C:\\Program Files\\Docker\\docker.exe".to_string()
        } else {
            "/usr/bin/docker".to_string()
        });
        config.harness_sandbox_image_digest = Some("mutable:latest".to_string());
        assert!(validation_config(&config, "sandbox").is_err());

        config.harness_sandbox_image_digest = Some(
            "harness@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        );
        assert!(validation_config(&config, "sandbox").is_ok());
    }

    #[test]
    fn validation_profiles_are_built_only_from_node_configuration() {
        let mut config = AgentConfig::default();
        config.harness_validation_profiles.insert(
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
