use crate::types::Backend;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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
            control_plane_url: "https://api.novusx.ai".to_string(),
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
    if home.exists() {
        return home;
    }

    let local = local_config_path();
    if local.exists() {
        return local;
    }

    home
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
    let data = serde_json::to_string_pretty(config).expect("config serialization");

    if let Some(path) = try_write(&config_path(), &data)? {
        return Ok(path);
    }

    if let Some(path) = try_write(&local_config_path(), &data)? {
        return Ok(path);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "unable to write config to home or local fallback",
    ))
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
