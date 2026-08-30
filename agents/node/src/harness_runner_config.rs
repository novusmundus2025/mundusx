use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerConfig {
    pub version: u32,
    pub runner_id: String,
    pub device_id: String,
    pub owner_user_id: String,
    #[serde(default)]
    pub tenant_ids: Vec<String>,
    pub control_plane_url: String,
    #[serde(default = "default_inference_model")]
    pub inference_model: String,
    #[serde(default = "default_runner_slots")]
    pub parallel_slots: u32,
    #[serde(default = "default_usable_memory_mb")]
    pub usable_memory_mb: u32,
    #[serde(default = "default_max_workspace_mb")]
    pub max_workspace_mb: u32,
    #[serde(default)]
    pub repositories: BTreeMap<String, String>,
    pub workspace_root: Option<String>,
    pub git_executable: Option<String>,
    #[serde(default)]
    pub validation_profiles: BTreeMap<String, HarnessValidationProfileConfig>,
    pub sandbox_runtime: Option<String>,
    pub sandbox_image_digest: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HarnessValidationProfileConfig {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub network_allowed: bool,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub max_memory_mb: u32,
    pub max_cpu_time_ms: u64,
    #[serde(default = "default_harness_max_processes")]
    pub max_processes: u32,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        Self {
            version: 1,
            runner_id: format!("runner-{}", &suffix[..16]),
            device_id: format!("runner-device-{}", &suffix[..16]),
            owner_user_id: String::new(),
            tenant_ids: Vec::new(),
            control_plane_url: "https://uat.mundusx.ai".to_string(),
            inference_model: default_inference_model(),
            parallel_slots: default_runner_slots(),
            usable_memory_mb: default_usable_memory_mb(),
            max_workspace_mb: default_max_workspace_mb(),
            repositories: BTreeMap::new(),
            workspace_root: None,
            git_executable: None,
            validation_profiles: BTreeMap::new(),
            sandbox_runtime: None,
            sandbox_image_digest: None,
        }
    }
}

fn default_inference_model() -> String {
    "mundusx-agnostic".to_string()
}

fn default_runner_slots() -> u32 {
    1
}

fn default_usable_memory_mb() -> u32 {
    4_096
}

fn default_max_workspace_mb() -> u32 {
    4_096
}

fn default_harness_max_processes() -> u32 {
    64
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("MUNDUSX_HARNESS_RUNNER_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".mundusx/harness-runner")))
        .unwrap_or_else(|| PathBuf::from(".mundusx/harness-runner"))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> std::io::Result<Option<RunnerConfig>> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

pub fn save_config(config: &RunnerConfig) -> std::io::Result<PathBuf> {
    let path = config_path();
    write_private_json(&path, config)?;
    Ok(path)
}

fn write_private_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    fs::write(path, body)
}
