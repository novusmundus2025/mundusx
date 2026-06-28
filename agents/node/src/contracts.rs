use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Auto,
    M,
    Cuda,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::M => "m",
            Self::Cuda => "cuda",
        }
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::Auto
    }
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "m" => Ok(Self::M),
            "cuda" => Ok(Self::Cuda),
            _ => Err("backend must be one of: auto, m, cuda".to_string()),
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Starting,
    Ready,
    Busy,
    Paused,
    Stopped,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Busy => "busy",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Assigned,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub node_id: String,
    pub public_key_fingerprint: String,
    pub public_key_hex: String,
    pub hostname: String,
    pub identity_trust_path: String,
    pub backend: Backend,
    pub contribution_percent: u8,
    pub capabilities: NodeCapabilityAdvertisement,
    pub agent_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Heartbeat {
    pub node_id: String,
    pub backend: Backend,
    pub agent_state: AgentState,
    pub available_memory_mb: u32,
    pub available_gpu_percent: u32,
    pub updated_at: String,
    pub contribution_percent: u8,
    pub hostname: String,
    pub identity_trust_path: String,
    pub power_source: String,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub policy_allowed: bool,
    pub policy_reason: Option<String>,
    pub worker_health: WorkerHealthReport,
    pub capabilities: NodeCapabilityAdvertisement,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRequest {
    pub request_id: String,
    pub prompt: String,
    pub preferred_backend: Backend,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub seed: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub request_id: String,
    pub prompt: String,
    pub preferred_backend: Backend,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub seed: Option<u64>,
    pub status: JobStatus,
    pub submitted_at: String,
    pub assigned_node_id: Option<String>,
    pub assigned_at: Option<String>,
    pub completed_at: Option<String>,
    pub worker_id: Option<String>,
    pub backend: Option<Backend>,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobClaimResponse {
    pub job: Option<JobRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobCompletion {
    pub job_id: String,
    pub node_id: String,
    pub worker_id: String,
    pub backend: Backend,
    pub status: JobStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerLaunchRequest {
    pub job_id: String,
    pub node_id: String,
    pub backend: Backend,
    pub prompt: String,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub seed: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerLaunchResponse {
    pub job_id: String,
    pub worker_id: String,
    pub status: String,
    pub output: String,
    pub error: Option<String>,
    pub backend: Backend,
    pub node_id: String,
    pub model: Option<String>,
    pub runtime_mode: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerHealthReport {
    pub healthy: bool,
    pub model_dir: String,
    pub model_name: Option<String>,
    pub model_path: Option<String>,
    pub llama_cli_available: bool,
    pub blas_device_available: bool,
    #[serde(default)]
    pub cuda_device_available: bool,
    #[serde(default)]
    pub cuda_driver_available: bool,
    #[serde(default)]
    pub cuda_device_name: Option<String>,
    #[serde(default)]
    pub cuda_memory_mb: Option<u32>,
    #[serde(default)]
    pub cuda_low_vram_profile: bool,
    pub power_source: String,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub runtime_mode: String,
    #[serde(default)]
    pub supported_runtime_modes: Vec<String>,
    pub checked_at: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCapability {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_vram_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCapabilityAdvertisement {
    pub backend: Backend,
    pub contribution_percent: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_vram_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usable_vram_mb: Option<u32>,
    pub runtime_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<ModelCapability>,
    pub ready_for_jobs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readiness_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerPolicyReport {
    pub allowed: bool,
    pub reason: Option<String>,
    pub power_source: String,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub recommended_max_contribution_percent: u8,
    pub checked_at: String,
    pub notes: Vec<String>,
}
