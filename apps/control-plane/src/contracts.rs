use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Auto,
    M,
    Cuda,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRequest {
    pub request_id: String,
    pub prompt: String,
    pub preferred_backend: Backend,
    pub model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub request_id: String,
    pub prompt: String,
    pub preferred_backend: Backend,
    pub model: Option<String>,
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
pub struct NodeRecord {
    pub node_id: String,
    pub public_key_fingerprint: String,
    pub backend: Backend,
    pub contribution_percent: u8,
    pub agent_version: String,
    pub state: AgentState,
    pub available_memory_mb: u32,
    pub available_gpu_percent: u32,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ControlPlaneSnapshot {
    pub nodes: Vec<NodeRecord>,
    pub jobs: Vec<JobRecord>,
    pub online_count: usize,
    pub paused_count: usize,
    pub stopped_count: usize,
    pub queued_job_count: usize,
    pub assigned_job_count: usize,
    pub completed_job_count: usize,
    pub failed_job_count: usize,
}
