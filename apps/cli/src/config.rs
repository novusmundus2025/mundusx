use crate::types::Backend;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A local LLM cluster the contributor already runs and has agreed to
/// contribute. When this is set the node serves work from that endpoint instead
/// of provisioning a MundusX runtime and downloading its own weights.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContributedCluster {
    pub kind: String,
    pub base_url: String,
    /// Scheduler capacity represented by this contributed endpoint. A detected
    /// cluster is external capacity, not the CLI host's remaining memory.
    #[serde(default = "default_cluster_capacity_class")]
    pub capacity_class: String,
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
    /// Capabilities the runtime advertises for the model, e.g. `tools`.
    #[serde(default)]
    pub model_capabilities: Vec<String>,
    /// Trained context length the runtime reports for the model.
    #[serde(default)]
    pub model_context_tokens: Option<u32>,
    /// Share of machine memory the runtime reports taking, e.g. vLLM's
    /// `gpu_memory_utilization`. Used as the basis for usable memory, because
    /// it is the portion of the machine the cluster genuinely occupies.
    #[serde(default)]
    pub memory_utilization: Option<f32>,
    /// Concurrent full-context sequences the runtime says it can hold.
    #[serde(default)]
    pub max_concurrency: Option<u32>,
    /// Configured scheduler ceiling (vLLM `max_num_seqs`), when reported.
    #[serde(default)]
    pub max_num_seqs: Option<u32>,
    /// True when the runtime has a tool-call parser loaded.
    #[serde(default)]
    pub supports_tool_calls: bool,
    #[serde(default)]
    pub adopted_at: Option<String>,
}

fn default_cluster_capacity_class() -> String {
    "server".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
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
    #[serde(default)]
    pub active_model: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_dir: Option<String>,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default)]
    pub runtime_preference: Option<String>,
    #[serde(default)]
    pub fallback_runtime: Option<String>,
    /// The running local cluster this node contributes, when one was adopted.
    #[serde(default)]
    pub contributed_cluster: Option<ContributedCluster>,
    /// Remembers a "no" so install/start stop asking on every run.
    #[serde(default)]
    pub cluster_prompt_declined: bool,
    /// Concurrent jobs this node will accept from the control plane.
    ///
    /// Set by the contributor. Runtime ceilings describe what a machine *could*
    /// hold; this is what its owner agreed to give away, so it wins over any
    /// derived figure and applies whether or not a cluster is contributed.
    #[serde(default)]
    pub max_jobs: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            device_id: format!("node-{}", uuid::Uuid::new_v4().simple()),
            public_key_fingerprint: None,
            profile_name: None,
            auth_token: None,
            connected: false,
            paused: false,
            backend_preference: Backend::Auto,
            contribution_percent: 0,
            control_plane_url: "https://uat.mundusx.ai".to_string(),
            active_model: None,
            models: vec![],
            model_dir: None,
            onboarding_completed: false,
            runtime_preference: None,
            fallback_runtime: None,
            contributed_cluster: None,
            cluster_prompt_declined: false,
            max_jobs: None,
        }
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

pub fn local_config_path() -> PathBuf {
    PathBuf::from(".opengpu").join("config.json")
}

pub fn resolved_config_path() -> PathBuf {
    if std::env::var_os("OPENGPU_HOME").is_some() {
        return config_path();
    }

    let home = config_path();
    let local = local_config_path();
    match (home.exists(), local.exists()) {
        (true, true) => {
            let home_modified = fs::metadata(&home).and_then(|meta| meta.modified()).ok();
            let local_modified = fs::metadata(&local).and_then(|meta| meta.modified()).ok();
            match (home_modified, local_modified) {
                (Some(home_time), Some(local_time)) => {
                    if local_time > home_time {
                        local
                    } else {
                        home
                    }
                }
                (None, Some(_)) => local,
                _ => home,
            }
        }
        (true, false) => home,
        (false, true) => local,
        (false, false) => home,
    }
}

pub fn load_config() -> std::io::Result<Option<Config>> {
    let path = resolved_config_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)?;
    let config = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(Some(config))
}

pub fn save_config(config: &Config) -> std::io::Result<PathBuf> {
    #[cfg(windows)]
    let serialized_config = {
        let mut config = config.clone();
        config.auth_token = None;
        config
    };
    #[cfg(not(windows))]
    let serialized_config = config.clone();
    let data = serde_json::to_string_pretty(&serialized_config).expect("config serialization");
    let mut last_success = None;
    let mut last_error = None;

    match try_write(&config_path(), &data) {
        Ok(path) => last_success = path.or(last_success),
        Err(error) => last_error = Some(error),
    }

    match try_write(&local_config_path(), &data) {
        Ok(path) => last_success = path.or(last_success),
        Err(error) => last_error = Some(error),
    }

    if let Some(path) = last_success {
        return Ok(path);
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unable to write config to home or local fallback",
        )
    }))
}

pub fn remove_config_files() -> std::io::Result<()> {
    remove_file_if_exists(&config_path())?;
    remove_file_if_exists(&local_config_path())?;
    Ok(())
}

pub fn config_exists() -> bool {
    resolved_config_path().exists()
}

fn try_write(path: &Path, data: &str) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::write(path, format!("{data}\n")) {
        Ok(()) => Ok(Some(path.to_path_buf())),
        Err(error) => {
            if path == config_path() {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
