use crate::contracts::Backend;
use crate::contracts::Heartbeat;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const HEARTBEAT_LOG_TTL_SECONDS: i64 = 30 * 60;

/// Mirror of the CLI's `ContributedCluster` so rewriting `config.json` from the
/// agent never drops the contributor's adopted-cluster decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributedCluster {
    pub kind: String,
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Parameter count of the advertised model, when the runtime reported it.
    /// Drives the node's capacity class instead of host memory.
    #[serde(default)]
    pub model_params: Option<u64>,
    /// On-disk size of the advertised model, used when params are unknown.
    #[serde(default)]
    pub model_bytes: Option<u64>,
    #[serde(default)]
    pub adopted_at: Option<String>,
}

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
    #[serde(default)]
    pub runtime_preference: Option<String>,
    #[serde(default)]
    pub fallback_runtime: Option<String>,
    #[serde(default)]
    pub contributed_cluster: Option<ContributedCluster>,
    #[serde(default)]
    pub cluster_prompt_declined: bool,
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
            control_plane_url: "https://uat.mundusx.ai".to_string(),
            model_dir: None,
            active_model: None,
            models: Vec::new(),
            runtime_preference: None,
            fallback_runtime: None,
            contributed_cluster: None,
            cluster_prompt_declined: false,
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

pub fn save_agent_config(config: &AgentConfig) -> std::io::Result<PathBuf> {
    save_json(config_path(), config)
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
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let pruned = reset_expired_heartbeat_log(&existing, &line, HEARTBEAT_LOG_TTL_SECONDS);
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?
        .write_all(pruned.as_bytes())?;
    Ok(path)
}

fn reset_expired_heartbeat_log(existing: &str, new_line: &str, ttl_seconds: i64) -> String {
    let Some(current_timestamp) = heartbeat_line_timestamp(new_line) else {
        return format!("{new_line}\n");
    };
    let valid_existing = existing
        .lines()
        .filter_map(|line| heartbeat_line_timestamp(line).map(|timestamp| (timestamp, line)))
        .collect::<Vec<_>>();

    let Some((first_timestamp, _)) = valid_existing.first() else {
        return format!("{new_line}\n");
    };

    if current_timestamp.saturating_sub(*first_timestamp) >= ttl_seconds {
        return format!("{new_line}\n");
    }

    let mut lines = valid_existing
        .into_iter()
        .map(|(_, line)| line.to_string())
        .collect::<Vec<_>>();
    lines.push(new_line.to_string());
    format!("{}\n", lines.join("\n"))
}

fn heartbeat_line_timestamp(line: &str) -> Option<i64> {
    let heartbeat: Heartbeat = serde_json::from_str(line).ok()?;
    heartbeat.updated_at.parse::<i64>().ok()
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
    use crate::contracts::{
        AgentState, ModelCapability, NodeCapabilityAdvertisement, NodeCapabilityProfile,
        WorkerHealthReport,
    };
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
            identity_trust_path: "keychain://mundusx".to_string(),
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
                llama_server_available: false,
                persistent_runtime_warm: false,
                persistent_runtime_url: None,
                runtime_kind: "batch".to_string(),
                runtime_preference: None,
                fallback_runtime: None,
                mlx_available: false,
                blas_device_available: true,
                cuda_device_available: false,
                cuda_driver_available: false,
                cuda_device_name: None,
                cuda_memory_mb: None,
                cuda_low_vram_profile: false,
                power_source: "ac".to_string(),
                on_battery: false,
                battery_percent: Some(100),
                runtime_mode: "native".to_string(),
                parallel_slots: 1,
                supported_runtime_modes: vec!["local".to_string()],
                capabilities: NodeCapabilityProfile::default(),
                checked_at: updated_at.to_string(),
                notes: Vec::new(),
            },
            capabilities: NodeCapabilityAdvertisement {
                schema_version: 2,
                backend: Backend::Auto,
                contribution_percent: 20,
                physical_memory_mb: Some(16_384),
                usable_memory_mb: Some(3_277),
                available_memory_mb: Some(12_000),
                physical_vram_mb: None,
                usable_vram_mb: None,
                runtime_mode: "native".to_string(),
                parallel_slots: 1,
                capacity_class: "micro".to_string(),
                supported_roles: vec![],
                supported_tools: vec![],
                active_model: Some(ModelCapability {
                    name: "llama3.1:8b".to_string(),
                    path: Some("/tmp/models/llama3.1-8b.gguf".to_string()),
                    format: Some("gguf".to_string()),
                    quantization: None,
                    size_bytes: None,
                    estimated_vram_mb: None,
                    compatibility: Some("accepted".to_string()),
                    compatibility_reason: None,
                }),
                ready_for_jobs: true,
                readiness_reason: None,
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

    #[test]
    fn save_heartbeat_resets_history_after_thirty_minutes() {
        with_temp_home(|| {
            save_heartbeat(&heartbeat("1000")).unwrap();
            save_heartbeat(&heartbeat("1700")).unwrap();
            save_heartbeat(&heartbeat("2801")).unwrap();

            let raw = fs::read_to_string(heartbeat_log_path()).unwrap();
            assert!(!raw.contains(r#""updated_at":"1000""#));
            assert!(!raw.contains(r#""updated_at":"1700""#));
            assert!(raw.contains(r#""updated_at":"2801""#));
        });
    }

    #[test]
    fn save_heartbeat_keeps_history_until_thirty_minutes() {
        with_temp_home(|| {
            save_heartbeat(&heartbeat("1000")).unwrap();
            save_heartbeat(&heartbeat("2799")).unwrap();

            let raw = fs::read_to_string(heartbeat_log_path()).unwrap();
            assert!(raw.contains(r#""updated_at":"1000""#));
            assert!(raw.contains(r#""updated_at":"2799""#));
        });
    }

    #[test]
    fn save_heartbeat_drops_malformed_history_entries() {
        with_temp_home(|| {
            fs::create_dir_all(config_dir()).unwrap();
            fs::write(heartbeat_log_path(), "not-json\n").unwrap();
            save_heartbeat(&heartbeat("5000")).unwrap();

            let raw = fs::read_to_string(heartbeat_log_path()).unwrap();
            assert!(!raw.contains("not-json"));
            assert!(raw.contains(r#""updated_at":"5000""#));
        });
    }
}
