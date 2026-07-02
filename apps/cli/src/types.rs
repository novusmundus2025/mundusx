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
    /// Whether the node's policy currently permits it to accept jobs.
    /// `None` means unknown; treated as allowed for backward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_allowed: Option<bool>,
    /// Whether the local worker is healthy and ready to run inference.
    /// `None` means unknown; treated as healthy for backward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_healthy: Option<bool>,
    /// Model identifiers available on this node (e.g. `["llama3.1:8b"]`).
    /// Empty means no model information was reported.
    #[serde(default)]
    pub models: Vec<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key_hex: Option<String>,
    pub hostname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_trust_path: Option<String>,
    pub backend: Backend,
    pub contribution_percent: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<NodeCapabilityAdvertisement>,
    pub agent_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerHealthReport {
    pub healthy: bool,
    pub model_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_path: Option<String>,
    pub llama_cli_available: bool,
    #[serde(default)]
    pub llama_server_available: bool,
    #[serde(default)]
    pub persistent_runtime_warm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent_runtime_url: Option<String>,
    #[serde(default)]
    pub runtime_kind: String,
    pub blas_device_available: bool,
    #[serde(default)]
    pub cuda_device_available: bool,
    #[serde(default)]
    pub cuda_driver_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_device_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cuda_memory_mb: Option<u32>,
    #[serde(default)]
    pub cuda_low_vram_profile: bool,
    pub power_source: String,
    pub on_battery: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_vram_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCapabilityAdvertisement {
    pub backend: Backend,
    pub contribution_percent: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_vram_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usable_vram_mb: Option<u32>,
    pub runtime_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<ModelCapability>,
    pub ready_for_jobs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness_reason: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_trust_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_battery: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_allowed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_health: Option<WorkerHealthReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<NodeCapabilityAdvertisement>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<Backend>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub node_id: String,
    pub status: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    #[test]
    fn backend_accepts_cuda_across_cli_parsing() {
        assert_eq!(
            <Backend as std::str::FromStr>::from_str("cuda").expect("parse cuda backend"),
            Backend::Cuda
        );
        assert_eq!(
            <Backend as ValueEnum>::from_str("cuda", false).expect("clap parse cuda backend"),
            Backend::Cuda
        );

        let variants = Backend::value_variants()
            .iter()
            .map(|variant| {
                variant
                    .to_possible_value()
                    .expect("possible value")
                    .get_name()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert!(
            variants.iter().any(|variant| variant == "cuda"),
            "expected clap variants to expose cuda"
        );
    }

    #[test]
    fn agent_registration_preserves_agent_contract_fields() {
        let registration: AgentRegistration = serde_json::from_str(
            r#"{
                "node_id": "node-1",
                "public_key_fingerprint": "fingerprint",
                "public_key_hex": "abcd",
                "hostname": "host-1",
                "identity_trust_path": "/tmp/trust",
                "backend": "m",
                "contribution_percent": 70,
                "agent_version": "0.1.0"
            }"#,
        )
        .expect("deserialize agent registration");

        let json = serde_json::to_value(registration).expect("serialize registration");
        assert_eq!(
            json.get("public_key_hex").and_then(|value| value.as_str()),
            Some("abcd")
        );
        assert_eq!(
            json.get("identity_trust_path")
                .and_then(|value| value.as_str()),
            Some("/tmp/trust")
        );
    }

    #[test]
    fn heartbeat_preserves_extended_agent_status_fields() {
        let heartbeat: Heartbeat = serde_json::from_str(
            r#"{
                "node_id": "node-1",
                "backend": "cuda",
                "agent_state": "paused",
                "available_memory_mb": 32768,
                "available_gpu_percent": 55,
                "updated_at": "2026-06-02T12:00:00Z",
                "contribution_percent": 40,
                "hostname": "host-1",
                "identity_trust_path": "/tmp/trust",
                "power_source": "Battery Power",
                "on_battery": true,
                "battery_percent": 42,
                "policy_allowed": false,
                "policy_reason": "battery saver",
                "worker_health": {
                    "healthy": false,
                    "model_dir": "/models",
                    "model_name": "llama",
                    "model_path": "/models/llama.gguf",
                    "llama_cli_available": true,
                    "blas_device_available": false,
                    "power_source": "Battery Power",
                    "on_battery": true,
                    "battery_percent": 42,
                    "runtime_mode": "cpu",
                    "checked_at": "2026-06-02T12:00:00Z",
                    "notes": ["using fallback"]
                }
            }"#,
        )
        .expect("deserialize heartbeat");

        let json = serde_json::to_value(heartbeat).expect("serialize heartbeat");
        assert_eq!(
            json.get("backend").and_then(|value| value.as_str()),
            Some("cuda")
        );
        assert_eq!(
            json.get("identity_trust_path")
                .and_then(|value| value.as_str()),
            Some("/tmp/trust")
        );
        assert_eq!(
            json.get("worker_health")
                .and_then(|value| value.get("runtime_mode"))
                .and_then(|value| value.as_str()),
            Some("cpu")
        );
    }

    #[test]
    fn worker_launch_response_preserves_worker_output_and_origin() {
        let response: WorkerLaunchResponse = serde_json::from_str(
            r#"{
                "job_id": "job-1",
                "worker_id": "worker-1",
                "status": "completed",
                "output": "hello world",
                "error": null,
                "backend": "m",
                "node_id": "node-1"
            }"#,
        )
        .expect("deserialize worker launch response");

        let json = serde_json::to_value(response).expect("serialize worker launch response");
        assert_eq!(
            json.get("output").and_then(|value| value.as_str()),
            Some("hello world")
        );
        assert_eq!(
            json.get("backend").and_then(|value| value.as_str()),
            Some("m")
        );
        assert_eq!(
            json.get("node_id").and_then(|value| value.as_str()),
            Some("node-1")
        );
    }
}
