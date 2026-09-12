use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const PUBLIC_CONTROL_PLANE_URL: &str = "https://uat.mundusx.ai";
const LEGACY_CONTROL_PLANE_URL: &str = "https://mundusx.ai";

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
    #[serde(default = "default_repository_source_patterns")]
    pub repository_source_patterns: Vec<String>,
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub projects_root: Option<String>,
    pub git_executable: Option<String>,
    pub github_cli: Option<String>,
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
        let runner_home = config_dir();
        let git_executable = find_executable(&["git.exe", "git"]);
        let github_cli = find_executable(&["gh.exe", "gh"]);
        let maven_executable = find_executable(&["mvn.cmd", "mvn"]);
        let mut validation_profiles = BTreeMap::new();
        if let Some(git) = git_executable.as_ref() {
            validation_profiles.insert(
                "repository-default".to_string(),
                HarnessValidationProfileConfig {
                    executable: git.clone(),
                    arguments: vec![
                        "diff".to_string(),
                        "--check".to_string(),
                        "HEAD".to_string(),
                    ],
                    working_directory: String::new(),
                    environment: BTreeMap::new(),
                    network_allowed: false,
                    timeout_ms: 120_000,
                    max_output_bytes: 1_048_576,
                    max_memory_mb: 1_024,
                    max_cpu_time_ms: 120_000,
                    max_processes: 16,
                },
            );
        }
        add_maven_profile(&mut validation_profiles, maven_executable);
        Self {
            version: 2,
            runner_id: format!("runner-{}", &suffix[..16]),
            device_id: format!("runner-device-{}", &suffix[..16]),
            owner_user_id: String::new(),
            tenant_ids: vec!["owner:any".to_string()],
            control_plane_url: PUBLIC_CONTROL_PLANE_URL.to_string(),
            inference_model: default_inference_model(),
            parallel_slots: default_runner_slots(),
            usable_memory_mb: default_usable_memory_mb(),
            max_workspace_mb: default_max_workspace_mb(),
            repositories: BTreeMap::new(),
            repository_source_patterns: default_repository_source_patterns(),
            workspace_root: Some(runner_home.join("workspaces").display().to_string()),
            projects_root: Some(default_projects_root().display().to_string()),
            git_executable,
            github_cli,
            validation_profiles,
            sandbox_runtime: None,
            sandbox_image_digest: None,
        }
    }
}

fn migrate_control_plane_url(url: &mut String) -> bool {
    if url.trim_end_matches('/') == LEGACY_CONTROL_PLANE_URL {
        *url = PUBLIC_CONTROL_PLANE_URL.to_string();
        true
    } else {
        false
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

fn default_repository_source_patterns() -> Vec<String> {
    vec!["local-project:".to_string(), "github:".to_string()]
}

fn default_projects_root() -> PathBuf {
    dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|directory| directory.join("documents")))
        .unwrap_or_else(|| PathBuf::from("documents"))
        .join("mundusx")
        .join("projects")
}

fn find_executable(names: &[&str]) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() && candidate.is_absolute() {
                return Some(candidate.display().to_string());
            }
        }
    }
    None
}

fn maven_validation_profile(maven: String) -> Option<HarnessValidationProfileConfig> {
    #[cfg(windows)]
    let (executable, arguments) = {
        let shell = std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_file())
            .map(|path| path.display().to_string())
            .or_else(|| find_executable(&["cmd.exe"]))?;
        (
            shell,
            vec![
                "/D".to_string(),
                "/C".to_string(),
                "call".to_string(),
                maven,
                "--batch-mode".to_string(),
                "test".to_string(),
            ],
        )
    };
    #[cfg(not(windows))]
    let (executable, arguments) = (maven, vec!["--batch-mode".to_string(), "test".to_string()]);
    Some(HarnessValidationProfileConfig {
        executable,
        arguments,
        working_directory: String::new(),
        environment: BTreeMap::new(),
        network_allowed: true,
        timeout_ms: 300_000,
        max_output_bytes: 2_097_152,
        max_memory_mb: 2_048,
        max_cpu_time_ms: 300_000,
        max_processes: 64,
    })
}

fn add_maven_profile(
    profiles: &mut BTreeMap<String, HarnessValidationProfileConfig>,
    maven: Option<String>,
) {
    if profiles.contains_key("java-maven-test") {
        return;
    }
    if let Some(profile) = maven.and_then(maven_validation_profile) {
        profiles.insert("java-maven-test".to_string(), profile);
    }
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
    let raw = fs::read_to_string(&path)?;
    let mut config: RunnerConfig = serde_json::from_str(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let migrated_control_plane = migrate_control_plane_url(&mut config.control_plane_url);
    add_maven_profile(
        &mut config.validation_profiles,
        find_executable(&["mvn.cmd", "mvn"]),
    );
    if config.projects_root.is_none() {
        config.projects_root = Some(default_projects_root().display().to_string());
    }
    if !config
        .repository_source_patterns
        .iter()
        .any(|pattern| pattern == "local-project:")
    {
        config
            .repository_source_patterns
            .insert(0, "local-project:".to_string());
    }
    config.version = config.version.max(2);
    if migrated_control_plane {
        write_private_json(&path, &config)?;
    }
    Ok(Some(config))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_only_the_retired_public_origin() {
        let mut legacy = LEGACY_CONTROL_PLANE_URL.to_string();
        assert!(migrate_control_plane_url(&mut legacy));
        assert_eq!(legacy, PUBLIC_CONTROL_PLANE_URL);

        let mut custom = "https://private.example.com".to_string();
        assert!(!migrate_control_plane_url(&mut custom));
    }

    #[test]
    fn maven_profile_uses_fixed_test_arguments_and_bounded_resources() {
        #[cfg(windows)]
        let maven = r"C:\tools\apache-maven\bin\mvn.cmd".to_string();
        #[cfg(not(windows))]
        let maven = "/opt/apache-maven/bin/mvn".to_string();
        let profile = maven_validation_profile(maven.clone()).expect("platform command available");
        assert!(profile
            .arguments
            .iter()
            .any(|argument| argument == "--batch-mode"));
        assert_eq!(profile.arguments.last().map(String::as_str), Some("test"));
        #[cfg(windows)]
        assert!(profile.arguments.iter().any(|argument| argument == &maven));
        #[cfg(not(windows))]
        assert_eq!(profile.executable, maven);
        assert!(profile.network_allowed);
        assert_eq!(profile.timeout_ms, 300_000);
        assert_eq!(profile.max_output_bytes, 2_097_152);
    }

    #[test]
    fn defaults_include_local_projects_without_removing_github() {
        let config = RunnerConfig::default();
        assert!(config.projects_root.is_some());
        assert!(config
            .repository_source_patterns
            .iter()
            .any(|pattern| pattern == "local-project:"));
        assert!(config
            .repository_source_patterns
            .iter()
            .any(|pattern| pattern == "github:"));
    }
}
