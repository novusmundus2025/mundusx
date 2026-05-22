use crate::contracts::Heartbeat;
use crate::contracts::Backend;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    pub version: u32,
    pub device_id: String,
    pub public_key_fingerprint: Option<String>,
    pub profile_name: Option<String>,
    pub auth_token: Option<String>,
    pub connected: bool,
    pub paused: bool,
    pub backend_preference: Backend,
    pub contribution_percent: u8,
    pub control_plane_url: String,
    /// Explicit path to model directory. Defaults to ~/.opengpu/models when absent.
    #[serde(default)]
    pub model_dir: Option<String>,
    #[serde(default)]
    pub active_model: Option<String>,
    /// Model identifiers this node has available (e.g. ["llama3.1:8b"]).
    #[serde(default)]
    pub models: Vec<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            version: 1,
            device_id: String::new(),
            public_key_fingerprint: None,
            profile_name: None,
            auth_token: None,
            connected: false,
            paused: false,
            backend_preference: Backend::Auto,
            contribution_percent: 0,
            control_plane_url: "https://api.novusx.ai".to_string(),
            model_dir: None,
            active_model: None,
            models: Vec::new(),
        }
    }
}

impl AgentConfig {
    /// Effective model directory: explicit path, or ~/.opengpu/models as fallback.
    pub fn effective_model_dir(&self) -> std::path::PathBuf {
        self.model_dir
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                    .join(".opengpu/models")
            })
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn agent_state_path() -> PathBuf {
    config_dir().join("agent-state.json")
}

pub fn heartbeat_log_path() -> PathBuf {
    config_dir().join("heartbeat.jsonl")
}

pub fn identity_path() -> PathBuf {
    config_dir().join("identity.json")
}

pub fn load_agent_config() -> std::io::Result<Option<AgentConfig>> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let config: AgentConfig = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(Some(config))
}

pub fn save_agent_state(heartbeat: &Heartbeat) -> std::io::Result<PathBuf> {
    save_json(agent_state_path(), heartbeat)
}

pub fn save_heartbeat(heartbeat: &Heartbeat) -> std::io::Result<PathBuf> {
    let path = heartbeat_log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let line = serde_json::to_string(heartbeat).expect("heartbeat serialization");
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(format!("{line}\n").as_bytes())?;
    Ok(path)
}

pub fn load_last_heartbeat() -> std::io::Result<Option<Heartbeat>> {
    let path = agent_state_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let heartbeat: Heartbeat = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(Some(heartbeat))
}

fn save_json<T: Serialize>(path: PathBuf, value: &T) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let data = serde_json::to_string_pretty(value).expect("json serialization");
    fs::write(&path, format!("{data}\n"))?;
    Ok(path)
}
