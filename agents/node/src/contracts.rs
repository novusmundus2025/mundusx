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
    Vulkan,
    Vllm,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::M => "m",
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
            Self::Vllm => "vllm",
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
            "vulkan" => Ok(Self::Vulkan),
            "vllm" => Ok(Self::Vllm),
            _ => Err("backend must be one of: auto, m, cuda, vulkan, vllm".to_string()),
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
    /// Control-plane-owned scheduler contract version. This is intentionally
    /// independent from the internal capability profile schema version.
    pub capability_fabric_version: String,
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
pub struct LocalSlotLeaseRequest {
    pub request_id: String,
    #[serde(default = "default_local_slot_count")]
    pub slots: u8,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_local_lease_ttl_seconds")]
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalSlotLeaseRenewRequest {
    pub lease_id: String,
    #[serde(default = "default_local_lease_ttl_seconds")]
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalSlotLeaseReleaseRequest {
    pub lease_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSlotLeaseRecord {
    pub lease_id: String,
    pub node_id: String,
    pub request_id: String,
    pub slots: u8,
    #[serde(default)]
    pub model: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalSlotLeaseResponse {
    pub granted: bool,
    #[serde(default)]
    pub lease: Option<LocalSlotLeaseRecord>,
    #[serde(default)]
    pub error: Option<String>,
    pub active_local_slots: usize,
    pub active_network_slots: usize,
    pub total_slots: usize,
    pub available_slots: usize,
}

fn default_local_slot_count() -> u8 {
    1
}

fn default_local_lease_ttl_seconds() -> u64 {
    30
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
    #[serde(default)]
    pub stream: bool,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
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
pub struct JobStreamDelta {
    pub job_id: String,
    pub node_id: String,
    pub assignment_id: String,
    pub sequence: u64,
    pub delta: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStreamAck {
    pub job_id: String,
    pub sequence: u64,
    pub accepted: bool,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerLaunchRequest {
    pub job_id: String,
    pub node_id: String,
    pub backend: Backend,
    #[serde(default)]
    pub stream: bool,
    pub prompt: String,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
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
    #[serde(default)]
    pub llama_server_available: bool,
    #[serde(default)]
    pub persistent_runtime_warm: bool,
    #[serde(default)]
    pub persistent_runtime_url: Option<String>,
    #[serde(default)]
    pub runtime_kind: String,
    #[serde(default)]
    pub runtime_preference: Option<String>,
    #[serde(default)]
    pub fallback_runtime: Option<String>,
    #[serde(default)]
    pub mlx_available: bool,
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
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u8,
    #[serde(default)]
    pub supported_runtime_modes: Vec<String>,
    #[serde(default)]
    pub streaming_supported: bool,
    #[serde(default)]
    pub capabilities: NodeCapabilityProfile,
    pub checked_at: String,
    pub notes: Vec<String>,
}

fn default_parallel_slots() -> u8 {
    1
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub warm: bool,
    #[serde(default)]
    pub context_tokens: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// How output admission is calculated. `context_window` means the runtime
    /// has no independent output ceiling: input, output, and safety overhead
    /// must fit inside the served context window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_capacity_mode: Option<String>,
    #[serde(default)]
    pub capacity_class: String,
    #[serde(default)]
    pub roles: Vec<NodeRole>,
    #[serde(default)]
    pub task_capabilities: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub specialties: Vec<String>,
    #[serde(default)]
    pub supports_structured_output: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    Chat,
    Coding,
    Vision,
    Embedding,
    ToolUse,
    ChunkAnalysis,
    Reducer,
    Synthesizer,
    Batch,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Coding => "coding",
            Self::Vision => "vision",
            Self::Embedding => "embedding",
            Self::ToolUse => "tool_use",
            Self::ChunkAnalysis => "chunk_analysis",
            Self::Reducer => "reducer",
            Self::Synthesizer => "synthesizer",
            Self::Batch => "batch",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NodeCapabilityProfile {
    #[serde(default = "default_capability_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub models: Vec<ModelCapability>,
    #[serde(default)]
    pub physical_memory_mb: Option<u32>,
    #[serde(default)]
    pub usable_memory_mb: Option<u32>,
    #[serde(default)]
    pub available_memory_mb: Option<u32>,
    #[serde(default)]
    pub capacity_class: String,
    #[serde(default)]
    pub max_context_tokens: Option<u32>,
    #[serde(default)]
    pub max_num_seqs: Option<u32>,
    #[serde(default)]
    pub kv_cache_size_tokens: Option<u64>,
    #[serde(default)]
    pub total_vram_mb: Option<u32>,
    #[serde(default)]
    pub available_vram_mb: Option<u32>,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_embeddings: bool,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default = "default_parallel_jobs")]
    pub max_parallel_jobs: u32,
    #[serde(default)]
    pub current_load_percent: Option<u8>,
    #[serde(default)]
    pub roles: Vec<NodeRole>,
    #[serde(default)]
    pub skill_tags: Vec<String>,
    #[serde(default)]
    pub supported_tools: Vec<String>,
}

impl Default for NodeCapabilityProfile {
    fn default() -> Self {
        Self {
            schema_version: 4,
            models: Vec::new(),
            physical_memory_mb: None,
            usable_memory_mb: None,
            available_memory_mb: None,
            capacity_class: String::new(),
            max_context_tokens: None,
            max_num_seqs: None,
            kv_cache_size_tokens: None,
            total_vram_mb: None,
            available_vram_mb: None,
            supports_vision: false,
            supports_embeddings: false,
            supports_tools: false,
            max_parallel_jobs: 1,
            current_load_percent: None,
            roles: Vec::new(),
            skill_tags: Vec::new(),
            supported_tools: Vec::new(),
        }
    }
}

fn default_parallel_jobs() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCapabilityAdvertisement {
    #[serde(default = "default_capability_schema_version")]
    pub schema_version: u32,
    pub backend: Backend,
    pub contribution_percent: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_memory_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usable_memory_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_memory_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_vram_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usable_vram_mb: Option<u32>,
    pub runtime_mode: String,
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u8,
    #[serde(default)]
    pub capacity_class: String,
    #[serde(default)]
    pub supported_roles: Vec<NodeRole>,
    #[serde(default)]
    pub supported_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessCapabilityAdvertisement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<ModelCapability>,
    pub ready_for_jobs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readiness_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HarnessCapabilityAdvertisement {
    pub execution_modes: Vec<String>,
    pub supported_operations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_runtime: Option<String>,
    pub network_default_disabled: bool,
    pub max_workspace_mb: u32,
}

fn default_capability_schema_version() -> u32 {
    4
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeAdmissionStatus {
    pub node_id: String,
    pub state: AgentState,
    pub policy_allowed: bool,
    pub policy_reason: Option<String>,
    #[serde(default)]
    pub computed_policy_allowed: bool,
    #[serde(default)]
    pub computed_policy_reason: Option<String>,
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
