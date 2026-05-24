use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
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

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::Auto
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

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Assigned => "assigned",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
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
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub stream: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionChoiceMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatCompletionChoiceMessage,
    pub finish_reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionOpenGpu {
    pub job_id: String,
    pub request_id: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub opengpu: ChatCompletionOpenGpu,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerHealthReport {
    pub healthy: bool,
    pub model_dir: String,
    pub model_name: Option<String>,
    pub model_path: Option<String>,
    pub llama_cli_available: bool,
    pub blas_device_available: bool,
    pub power_source: String,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub runtime_mode: String,
    pub checked_at: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobEventRecord {
    pub id: u64,
    pub node_id: Option<String>,
    pub job_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreditsLedgerRecord {
    pub id: String,
    pub user_id: Option<String>,
    pub device_id: Option<String>,
    pub job_id: Option<String>,
    pub entry_type: String,
    pub amount: f64,
    pub currency: String,
    pub metadata: serde_json::Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRecord {
    pub node_id: String,
    pub public_key_fingerprint: String,
    #[serde(default)]
    pub public_key_hex: String,
    #[serde(default)]
    pub hostname: String,
    pub backend: Backend,
    pub contribution_percent: u8,
    pub agent_version: String,
    pub state: AgentState,
    pub available_memory_mb: u32,
    pub available_gpu_percent: u32,
    pub identity_trust_path: String,
    pub power_source: String,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub policy_allowed: bool,
    pub policy_reason: Option<String>,
    pub worker_health: Option<WorkerHealthReport>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ControlPlaneSnapshot {
    pub nodes: Vec<NodeRecord>,
    pub jobs: Vec<JobRecord>,
    pub job_events: usize,
    pub credits_ledger: usize,
    pub credits_total: f64,
    pub credits_by_node: BTreeMap<String, f64>,
    pub storage_source: String,
    pub online_count: usize,
    pub paused_count: usize,
    pub policy_blocked_count: usize,
    pub stopped_count: usize,
    pub queued_job_count: usize,
    pub assigned_job_count: usize,
    pub completed_job_count: usize,
    pub failed_job_count: usize,
}
