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
    let log_path = heartbeat_log_path();
    if log_path.exists() {
        let raw = fs::read_to_string(&log_path)?;
        if let Some(line) = raw.lines().rev().find(|line| !line.trim().is_empty()) {
            let heartbeat: Heartbeat = serde_json::from_str(line)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            return Ok(Some(heartbeat));
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{AgentState, WorkerHealthReport};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn with_temp_home(test: impl FnOnce()) {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let temp = std::env::temp_dir().join(format!(
            "opengpu-node-agent-storage-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let previous = std::env::var_os("OPENGPU_HOME");
        std::env::set_var("OPENGPU_HOME", &temp);
        test();
        match previous {
            Some(value) => std::env::set_var("OPENGPU_HOME", value),
            None => std::env::remove_var("OPENGPU_HOME"),
        }
        let _ = fs::remove_dir_all(temp);
    }

    fn heartbeat(updated_at: &str) -> Heartbeat {
        Heartbeat {
            node_id: "node-1".to_string(),
            backend: Backend::Auto,
            agent_state: AgentState::Ready,
            available_memory_mb: 4096,
            available_gpu_percent: 80,
            updated_at: updated_at.to_string(),
            contribution_percent: 20,
            hostname: "host".to_string(),
            identity_trust_path: "keychain://novusx".to_string(),
            power_source: "ac".to_string(),
            on_battery: false,
            battery_percent: Some(100),
            policy_allowed: true,
            policy_reason: None,
            worker_health: WorkerHealthReport {
                healthy: true,
                model_dir: "/tmp/models".to_string(),
                model_name: Some("llama3.1:8b".to_string()),
                model_path: Some("/tmp/models/llama3.1-8b.gguf".to_string()),
                llama_cli_available: true,
                blas_device_available: true,
                power_source: "ac".to_string(),
                on_battery: false,
                battery_percent: Some(100),
                runtime_mode: "native".to_string(),
                checked_at: updated_at.to_string(),
                notes: Vec::new(),
            },
        }
    }

    #[test]
    fn load_last_heartbeat_prefers_latest_log_entry() {
        with_temp_home(|| {
            save_agent_state(&heartbeat("10")).unwrap();
            save_heartbeat(&heartbeat("11")).unwrap();
            save_heartbeat(&heartbeat("12")).unwrap();

            let restored = load_last_heartbeat().unwrap().expect("heartbeat");
            assert_eq!(restored.updated_at, "12");
        });
    }

    #[test]
    fn load_last_heartbeat_falls_back_to_agent_state() {
        with_temp_home(|| {
            save_agent_state(&heartbeat("20")).unwrap();

            let restored = load_last_heartbeat().unwrap().expect("heartbeat");
            assert_eq!(restored.updated_at, "20");
        });
    }
}
