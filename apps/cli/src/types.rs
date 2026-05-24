use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Auto,
    M,
    #[value(skip)]
    Cuda,
}

impl Default for Backend {
    fn default() -> Self {
        Self::Auto
    }
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::M => "m",
            Self::Cuda => "cuda",
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Self::Auto)
    }
}

impl FromStr for Backend {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "m" => Ok(Self::M),
            _ => Err("backend must be one of: auto or m".to_string()),
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
pub enum NodeState {
    Online,
    Busy,
    Idle,
    Offline,
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

impl NodeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Idle => "idle",
            Self::Offline => "offline",
        }
    }
}

impl fmt::Display for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeStatus {
    pub node_id: String,
    pub backend: Backend,
    pub state: NodeState,
    pub available_memory_mb: u32,
    pub available_gpu_percent: u32,
    pub label: String,
    pub region: Option<String>,
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
pub struct RoutingDecision {
    pub request_id: String,
    pub selected_node_id: Option<String>,
    pub selected_backend: Option<Backend>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub node_id: String,
    pub public_key_fingerprint: String,
    pub hostname: String,
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
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub node_id: String,
    pub status: String,
    pub updated_at: String,
}
