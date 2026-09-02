mod auth_token;
mod cluster;
mod config;
mod identity;
mod model;
mod model_catalog;
mod routing;
mod theme;
mod types;
mod updater;

use clap::{Parser, Subcommand, ValueEnum};
use crossterm::cursor::MoveTo;
use crossterm::event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::Color;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use types::{Backend, Heartbeat};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ExecutionMode {
    Single,
    Auto,
    Decompose,
}

impl ExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Auto => "auto",
            Self::Decompose => "decompose",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum RequestRoutingMode {
    LocalFirst,
    LocalOnly,
    NetworkOnly,
}

use cluster::{ClusterPromptDecision, DetectedCluster};
use config::{
    config_dir, config_exists, load_config, resolved_config_path, save_config, Config,
    ContributedCluster,
};
use identity::{device_id_for_identity, ensure_identity, load_identity, load_or_create_identity};
use model::{
    active_model_name, add_model, configured_model_dir_string, ensure_catalog_model_fits,
    ensure_effective_model_dir, import_model, list_models, prune_models, remove_model, use_model,
    ImportModelOptions, ModelRecord,
};
use model_catalog::{
    lookup_model_for_backend, selectable_catalog_options_for, selectable_options_for,
    selection_for, ModelOption,
};

const PUBLIC_CONTROL_PLANE_URL: &str = "https://uat.mundusx.ai";

#[derive(Parser, Debug)]
#[command(
    name = "opengpu",
    version,
    about = "MundusX CLI",
    arg_required_else_help = true
)]
struct Cli {
    /// Human-readable output theme. JSON output is never styled.
    #[arg(long, global = true, value_enum, default_value_t = theme::ThemeSelection::Auto)]
    theme: theme::ThemeSelection,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Guided first-run setup for this contributor machine
    Install {
        /// Use the public MundusX control plane
        #[arg(long)]
        public: bool,
        /// Configure a private/custom control plane
        #[arg(long)]
        private: bool,
        /// Private/custom control-plane URL
        #[arg(long)]
        control_plane_url: Option<String>,
        /// Contribution cap to save without opening the selector
        #[arg(long)]
        cap_percent: Option<u8>,
        /// Contribute a detected running local LLM cluster without being asked
        #[arg(long, conflicts_with = "no_contribute_cluster")]
        contribute_cluster: bool,
        /// Never contribute a detected running local LLM cluster
        #[arg(long, conflicts_with = "contribute_cluster")]
        no_contribute_cluster: bool,
        /// Probe this cluster endpoint instead of the well-known local ports
        #[arg(long)]
        cluster_url: Option<String>,
        /// Concurrent jobs to accept on a contributed cluster (skips the prompt)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
        max_jobs: Option<u32>,
    },
    /// Start the MundusX network
    Start {
        /// Run the node agent in the background and return after verified startup
        #[arg(long, conflicts_with = "debug")]
        background: bool,
        /// Run the foreground session with explicit diagnostic labeling
        #[arg(long, conflicts_with = "background")]
        debug: bool,
        /// Contribute a detected running local LLM cluster without being asked
        #[arg(long, conflicts_with = "no_contribute_cluster")]
        contribute_cluster: bool,
        /// Never contribute a detected running local LLM cluster
        #[arg(long, conflicts_with = "contribute_cluster")]
        no_contribute_cluster: bool,
        /// Probe this cluster endpoint instead of the well-known local ports
        #[arg(long)]
        cluster_url: Option<String>,
        /// Concurrent jobs to accept on a contributed cluster (skips the prompt)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
        max_jobs: Option<u32>,
    },
    /// Join the MundusX network (boots local state on first use)
    Connect,
    /// Pause contribution while keeping this device connected
    Pause,
    /// Resume contribution in the background
    Resume,
    /// Store local operator auth state
    Login {
        #[arg(long)]
        token: Option<String>,
    },
    /// Clear local operator auth state
    Logout,
    /// Review or complete contributor onboarding
    Onboarding {
        #[arg(long)]
        complete: bool,
        #[arg(long)]
        reset: bool,
    },
    /// Set or review the contributor cap
    Cap {
        /// Set the cap directly without using the selector
        value: Option<u8>,
        /// Set the cap directly instead of using the selector
        #[arg(long)]
        percent: Option<u8>,
        /// Clear the saved contribution cap
        #[arg(long)]
        reset: bool,
    },
    /// Leave the MundusX network
    #[command(visible_alias = "exit")]
    Disconnect,
    /// Show current node status
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Inspect config paths and writability
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Show local log source information
    Logs {
        #[arg(long)]
        json: bool,
    },
    /// Manage the local model cache
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },
    /// Manage local runtime adapters
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommands,
    },
    /// Inspect or change the running local LLM cluster this node contributes
    Cluster {
        #[command(subcommand)]
        command: ClusterCommands,
    },
    /// Update the MundusX binary
    Update,
    /// Show earned credits from the control plane
    Credits {
        #[arg(long)]
        json: bool,
        /// Entries per local log page
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Page number within the current local credit log, newest first
        #[arg(long, default_value_t = 0)]
        page: usize,
    },
    /// Submit and inspect async control-plane jobs
    Jobs {
        #[command(subcommand)]
        command: JobsCommands,
    },
    /// Get or set configuration values
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Run an inference request locally first, with control-plane fallback
    Run {
        /// The prompt to send
        #[arg(long, short = 'p')]
        prompt: String,
        /// Model name to use (defaults to active model)
        #[arg(long, short = 'm')]
        model: Option<String>,
        /// Preferred backend
        #[arg(long, default_value = "auto")]
        backend: Backend,
        /// Maximum tokens to generate (auto-selected when omitted)
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Let the control plane split eligible work into validated subjobs
        #[arg(long)]
        decompose: bool,
        /// Control whether the control plane may decompose work
        #[arg(long, value_enum)]
        execution_mode: Option<ExecutionMode>,
        /// Choose local-first, local-only, or network-only routing
        #[arg(long, value_enum, default_value_t = RequestRoutingMode::LocalFirst)]
        routing: RequestRoutingMode,
        /// Maximum seconds to wait for the control-plane job
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// Remote fallback poll interval in seconds
        #[arg(long, default_value_t = 2)]
        interval: u64,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Set the directory to search for GGUF model files
    #[command(name = "model-dir")]
    ModelDir { path: String },
    /// Set the control plane URL
    #[command(name = "control-plane-url")]
    ControlPlaneUrl { url: String },
}

#[derive(Subcommand, Debug)]
enum ModelCommands {
    /// List cached models
    List {
        #[arg(long)]
        json: bool,
    },
    /// Download or cache a model and mark it active
    Use { name: Option<String> },
    /// Download or cache a model without switching to it
    Add { name: Option<String> },
    /// Import an existing local model file and record compatibility metadata
    Import {
        path: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        backend: Option<Backend>,
        #[arg(long)]
        vram_mb: Option<u64>,
        #[arg(long)]
        activate: bool,
    },
    /// Remove a cached model
    Remove {
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// Remove inactive cached models
    Prune {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
enum ClusterCommands {
    /// Probe the local ports for a running LLM cluster and print what answered
    Scan {
        /// Probe this endpoint instead of the well-known local ports
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Contribute a running cluster without waiting for the install prompt
    Use {
        /// Cluster endpoint, e.g. http://127.0.0.1:11434
        url: String,
        /// Model this node should advertise from the cluster
        #[arg(long)]
        model: Option<String>,
        /// Concurrent jobs to accept on this cluster
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=64))]
        max_jobs: Option<u32>,
    },
    /// Stop contributing the recorded cluster and allow the prompt again
    Forget,
}

#[derive(Subcommand, Debug)]
enum RuntimeCommands {
    /// Install and enable a local runtime adapter
    Install {
        #[arg(value_enum)]
        runtime: RuntimeSelection,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum RuntimeSelection {
    Mlx,
}

impl RuntimeSelection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mlx => "mlx",
        }
    }
}

#[derive(Subcommand, Debug)]
enum JobsCommands {
    /// Submit an async job and print its job/request identifiers
    Submit {
        /// The prompt to send
        #[arg(long, short = 'p')]
        prompt: String,
        /// Model name to request
        #[arg(long, short = 'm')]
        model: Option<String>,
        /// Preferred backend
        #[arg(long, default_value = "auto")]
        backend: Backend,
        /// Maximum tokens to generate (auto-selected when omitted)
        #[arg(long)]
        max_tokens: Option<u32>,
        /// Let the control plane split eligible work into validated subjobs
        #[arg(long)]
        decompose: bool,
        /// Control whether the control plane may decompose work
        #[arg(long, value_enum)]
        execution_mode: Option<ExecutionMode>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Fetch current state for a submitted job
    Status {
        job_id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Poll until a job reaches a terminal state or times out
    Wait {
        job_id: String,
        /// Maximum seconds to wait
        #[arg(long, default_value_t = 300)]
        timeout: u64,
        /// Poll interval in seconds
        #[arg(long, default_value_t = 2)]
        interval: u64,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn current_config_or_default() -> Config {
    load_config().ok().flatten().unwrap_or_default()
}

fn clear_menu_screen() {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, Clear(ClearType::All), MoveTo(0, 0));
}

fn handles_terminal_key(kind: KeyEventKind) -> bool {
    kind != KeyEventKind::Release
}

macro_rules! raw_println {
    () => {
        print!("\r\n")
    };
    ($($arg:tt)*) => {
        print!("{}\r\n", format_args!($($arg)*))
    };
}

fn drain_pending_terminal_events() {
    while matches!(poll(Duration::from_millis(0)), Ok(true)) {
        if read().is_err() {
            break;
        }
    }
}

fn resolved_backend(config: &Config) -> Backend {
    if config.backend_preference.is_auto() {
        detect_backend()
    } else {
        config.backend_preference
    }
}

fn display_public_key_fingerprint(config: &Config) -> String {
    config
        .public_key_fingerprint
        .clone()
        .or_else(|| {
            load_identity()
                .ok()
                .flatten()
                .map(|identity| identity.fingerprint)
        })
        .or_else(identity_metadata_fingerprint)
        .unwrap_or_else(|| "unset".to_string())
}

fn display_public_key_hex(config: &Config) -> String {
    if let Some(public_key) = config.public_key_fingerprint.as_ref() {
        if let Ok(Some(identity)) = load_identity() {
            if identity.fingerprint == *public_key {
                return identity.public_key_hex;
            }
        }
    }

    load_identity()
        .ok()
        .flatten()
        .map(|identity| identity.public_key_hex)
        .or_else(identity_metadata_public_key_hex)
        .unwrap_or_else(|| "unset".to_string())
}

fn identity_metadata_value(field: &str) -> Option<String> {
    let text = std::fs::read_to_string(identity::resolved_identity_path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get(field)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.to_string())
}

fn identity_metadata_public_key_hex() -> Option<String> {
    identity_metadata_value("public_key_hex")
}

fn identity_metadata_fingerprint() -> Option<String> {
    identity_metadata_value("fingerprint")
}

fn run_nvidia_smi_query(args: &[&str]) -> Result<String, String> {
    let output = Command::new("nvidia-smi")
        .args(args)
        .output()
        .map_err(|error| format!("nvidia-smi unavailable: {error}"))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "nvidia-smi exited {}: {}",
        output.status,
        stderr.trim()
    ))
}

fn first_nvidia_smi_value(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn max_nvidia_smi_u64(stdout: &str) -> Option<u64> {
    stdout
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()
}

fn cuda_doctor_payload(
    os: &str,
    backend: Backend,
    name_query: Result<String, String>,
    memory_query: Result<String, String>,
) -> serde_json::Value {
    let gpu_name = name_query.as_deref().ok().and_then(first_nvidia_smi_value);
    let memory_mb = memory_query.as_deref().ok().and_then(max_nvidia_smi_u64);
    let nvidia_smi_available = name_query.is_ok() || memory_query.is_ok();
    let cuda_device_available = gpu_name.is_some() || memory_mb.is_some();
    let low_vram_profile = memory_mb.map(|memory| memory <= 4096).unwrap_or(false);
    let driver_available = nvidia_smi_available;
    let mut notes = Vec::new();

    if !nvidia_smi_available {
        notes.push(
            "nvidia-smi is unavailable; install or repair the NVIDIA driver before CUDA jobs can be advertised"
                .to_string(),
        );
    } else if !cuda_device_available {
        notes.push(
            "nvidia-smi responded, but no NVIDIA GPU rows were reported; this node should not advertise CUDA readiness"
                .to_string(),
        );
    }

    if low_vram_profile {
        notes.push(
            "low-VRAM CUDA profile selected; keep community workloads within the cap-applied model budget"
                .to_string(),
        );
    }

    if os == "windows" {
        notes.push(
            "LM Studio can be used on Windows without the full CUDA developer toolkit when its local OpenAI-compatible endpoint and loaded model probe successfully"
                .to_string(),
        );
    }

    if backend != Backend::Cuda {
        notes.push(format!(
            "selected backend is {}; CUDA diagnostics are informational unless CUDA is selected",
            backend.as_str()
        ));
    }

    let readiness = match (
        backend,
        nvidia_smi_available,
        cuda_device_available,
        memory_mb,
    ) {
        (Backend::Cuda, false, _, _) => "blocked-nvidia-smi-unavailable",
        (Backend::Cuda, true, false, _) => "blocked-no-nvidia-gpu",
        (Backend::Cuda, true, true, None) => "needs-vram-confirmation",
        (Backend::Cuda, true, true, Some(_)) => "cuda-prerequisites-detected",
        _ => "informational",
    };

    serde_json::json!({
        "os": os,
        "selected_backend": backend.as_str(),
        "nvidia_smi_available": nvidia_smi_available,
        "nvidia_driver_available": driver_available,
        "cuda_device_available": cuda_device_available,
        "cuda_device_name": gpu_name,
        "cuda_vram_mb": memory_mb,
        "cuda_low_vram_profile": low_vram_profile,
        "runtime_readiness": readiness,
        "lm_studio_without_cuda_toolkit_supported": os == "windows",
        "notes": notes,
        "name_probe_error": name_query.err(),
        "memory_probe_error": memory_query.err(),
    })
}

fn live_cuda_doctor_payload(backend: Backend) -> serde_json::Value {
    cuda_doctor_payload(
        env::consts::OS,
        backend,
        run_nvidia_smi_query(&["--query-gpu=name", "--format=csv,noheader"]),
        run_nvidia_smi_query(&["--query-gpu=memory.total", "--format=csv,noheader,nounits"]),
    )
}

fn command_available(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn vllm_doctor_payload(
    os: &str,
    backend: Backend,
    docker_available: bool,
    runtime_config_available: bool,
    nvidia_smi_available: bool,
    endpoint_healthy: bool,
) -> serde_json::Value {
    let supported_os = os == "linux";
    let selected = backend == Backend::Vllm;
    let mut notes = Vec::new();

    if !supported_os {
        notes.push(
            "vLLM is currently supported only for Linux/Ubuntu NVIDIA nodes; Windows keeps using the llama.cpp runtime path"
                .to_string(),
        );
    }

    if supported_os && !docker_available {
        notes.push(
            "Docker is unavailable to the current user; the MundusX vLLM runtime requires Docker daemon access"
                .to_string(),
        );
    }

    if supported_os && docker_available && !runtime_config_available {
        notes.push(
            "the pinned vLLM runtime is not configured; run install.sh --with-vllm".to_string(),
        );
    }

    if supported_os && !nvidia_smi_available {
        notes.push(
            "nvidia-smi is unavailable; vLLM NVIDIA nodes require a visible GPU driver".to_string(),
        );
    }

    if !selected {
        notes.push(format!(
            "selected backend is {}; vLLM diagnostics are informational unless vLLM is selected",
            backend.as_str()
        ));
    }
    if supported_os && runtime_config_available && !endpoint_healthy {
        notes.push(
            "the vLLM runtime is installed but its localhost endpoint is not healthy".to_string(),
        );
    }

    let readiness = if !selected {
        "informational"
    } else if !supported_os {
        "unsupported-on-this-os"
    } else if !docker_available {
        "blocked-docker-unavailable"
    } else if !runtime_config_available {
        "blocked-runtime-unconfigured"
    } else if !nvidia_smi_available {
        "blocked-nvidia-smi-unavailable"
    } else if !endpoint_healthy {
        "runtime-installed-not-running"
    } else {
        "vllm-runtime-ready"
    };

    serde_json::json!({
        "os": os,
        "selected_backend": backend.as_str(),
        "supported_os": supported_os,
        "docker_available": docker_available,
        "runtime_config_available": runtime_config_available,
        "nvidia_smi_available": nvidia_smi_available,
        "endpoint_healthy": endpoint_healthy,
        "runtime_readiness": readiness,
        "notes": notes,
    })
}

fn live_vllm_doctor_payload(backend: Backend) -> serde_json::Value {
    let docker_available = command_available("docker", &["info"]);
    let runtime_config_available = config::config_dir()
        .join("runtimes")
        .join("vllm")
        .join("runtime.conf")
        .is_file();
    let nvidia_smi_available =
        run_nvidia_smi_query(&["--query-gpu=name", "--format=csv,noheader"]).is_ok();
    let endpoint_healthy = ureq::get("http://127.0.0.1:8000/health")
        .timeout(Duration::from_secs(2))
        .call()
        .map(|response| response.status() < 500)
        .unwrap_or(false);

    vllm_doctor_payload(
        env::consts::OS,
        backend,
        docker_available,
        runtime_config_available,
        nvidia_smi_available,
        endpoint_healthy,
    )
}

fn doctor_payload(config: &Config) -> serde_json::Value {
    let resolved_path = resolved_config_path();
    let local_path = config::local_config_path();
    let home_path = config::config_path();
    let config_dir = config::config_dir();
    let model_dir = model::effective_model_dir(config);
    let backend = resolved_backend(config);
    let cuda = live_cuda_doctor_payload(backend);
    let vllm = live_vllm_doctor_payload(backend);

    serde_json::json!({
        "config_dir": config_dir,
        "resolved_config_path": resolved_path,
        "home_config_path": home_path,
        "local_config_path": local_path,
        "resolved_config_exists": resolved_path.exists(),
        "config_dir_exists": config_dir.exists(),
        "config_dir_writable": std::fs::create_dir_all(&config_dir).is_ok(),
        "resolved_config_parent_writable": resolved_path.parent().map(|parent| std::fs::create_dir_all(parent).is_ok()).unwrap_or(false),
        "identity_path": identity::resolved_identity_path(),
        "model_dir": model_dir,
        "model_dir_exists": model_dir.exists(),
        "model_dir_writable": std::fs::create_dir_all(&model_dir).is_ok(),
        "active_model": effective_active_model(config),
        "contributed_cluster": config.contributed_cluster,
        "auth_token_present": auth_token::operator_token_present(config),
        "cuda": cuda,
        "vllm": vllm,
    })
}

fn logs_payload() -> serde_json::Value {
    let config_dir = config::config_dir();
    let heartbeat_log_path = config_dir.join("heartbeat.jsonl");
    let agent_state_path = config_dir.join("agent-state.json");

    serde_json::json!({
        "config_dir": config_dir,
        "agent_state_path": agent_state_path,
        "agent_state_exists": agent_state_path.exists(),
        "heartbeat_log_path": heartbeat_log_path,
        "heartbeat_log_exists": heartbeat_log_path.exists(),
    })
}

fn print_doctor_report(config: &Config, json: bool) {
    let payload = doctor_payload(config);
    if json {
        if let Err(error) = print_json(&payload) {
            eprintln!("failed to print json: {error}");
            std::process::exit(1);
        }
        return;
    }

    theme::section("Doctor");
    theme::field(
        "configDir",
        payload["config_dir"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "resolvedConfigPath",
        payload["resolved_config_path"]
            .as_str()
            .unwrap_or("unknown"),
    );
    theme::field(
        "homeConfigPath",
        payload["home_config_path"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "localConfigPath",
        payload["local_config_path"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "resolvedConfigExists",
        theme::boolean(
            payload["resolved_config_exists"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "configDirWritable",
        theme::boolean(
            payload["config_dir_writable"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "resolvedConfigParentWritable",
        theme::boolean(
            payload["resolved_config_parent_writable"]
                .as_bool()
                .unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "identityPath",
        payload["identity_path"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "modelDir",
        payload["model_dir"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "modelDirWritable",
        theme::boolean(
            payload["model_dir_writable"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "activeModel",
        payload["active_model"].as_str().unwrap_or("none"),
    );
    theme::field(
        "authTokenPresent",
        theme::boolean(
            payload["auth_token_present"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    let cuda = &payload["cuda"];
    theme::section("CUDA diagnostics");
    theme::field(
        "cuda.selectedBackend",
        cuda["selected_backend"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "cuda.nvidiaSmiAvailable",
        theme::boolean(
            cuda["nvidia_smi_available"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "cuda.driverAvailable",
        theme::boolean(
            cuda["nvidia_driver_available"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "cuda.deviceAvailable",
        theme::boolean(
            cuda["cuda_device_available"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "cuda.deviceName",
        cuda["cuda_device_name"].as_str().unwrap_or("none"),
    );
    theme::field(
        "cuda.vramMb",
        cuda["cuda_vram_mb"]
            .as_u64()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
    );
    theme::field(
        "cuda.lowVramProfile",
        theme::boolean(
            cuda["cuda_low_vram_profile"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "cuda.runtimeReadiness",
        theme::status(cuda["runtime_readiness"].as_str().unwrap_or("unknown")),
    );
    if let Some(notes) = cuda["notes"].as_array() {
        for note in notes.iter().filter_map(|note| note.as_str()) {
            theme::note(format!("cuda: {note}"));
        }
    }

    let vllm = &payload["vllm"];
    theme::section("vLLM diagnostics");
    theme::field(
        "vllm.selectedBackend",
        vllm["selected_backend"].as_str().unwrap_or("unknown"),
    );
    theme::field(
        "vllm.supportedOs",
        theme::boolean(vllm["supported_os"].as_bool().unwrap_or(false), "yes", "no"),
    );
    theme::field(
        "vllm.dockerAvailable",
        theme::boolean(
            vllm["docker_available"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "vllm.runtimeConfigured",
        theme::boolean(
            vllm["runtime_config_available"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "vllm.endpointHealthy",
        theme::boolean(
            vllm["endpoint_healthy"].as_bool().unwrap_or(false),
            "yes",
            "no",
        ),
    );
    theme::field(
        "vllm.runtimeReadiness",
        theme::status(vllm["runtime_readiness"].as_str().unwrap_or("unknown")),
    );
    if let Some(notes) = vllm["notes"].as_array() {
        for note in notes.iter().filter_map(|note| note.as_str()) {
            theme::note(format!("vllm: {note}"));
        }
    }
}

fn print_logs_report(json: bool) {
    let payload = logs_payload();
    if json {
        if let Err(error) = print_json(&payload) {
            eprintln!("failed to print json: {error}");
            std::process::exit(1);
        }
        return;
    }

    println!(
        "configDir: {}",
        payload["config_dir"].as_str().unwrap_or("unknown")
    );
    println!(
        "agentStatePath: {}",
        payload["agent_state_path"].as_str().unwrap_or("unknown")
    );
    println!(
        "agentStateExists: {}",
        if payload["agent_state_exists"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "heartbeatLogPath: {}",
        payload["heartbeat_log_path"].as_str().unwrap_or("unknown")
    );
    println!(
        "heartbeatLogExists: {}",
        if payload["heartbeat_log_exists"].as_bool().unwrap_or(false) {
            "yes"
        } else {
            "no"
        }
    );
}

fn identity_ready() -> bool {
    load_identity().ok().flatten().is_some()
}

fn config_from_identity(identity: &identity::DeviceIdentity) -> Config {
    let mut config = Config::default();
    sync_config_identity(&mut config, identity);
    config
}

fn sync_config_identity(config: &mut Config, identity: &identity::DeviceIdentity) {
    config.device_id = device_id_for_identity(identity);
    config.public_key_fingerprint = Some(identity.fingerprint.clone());
}

// ---------------------------------------------------------------------------
// Control-plane inference
// ---------------------------------------------------------------------------

struct InferenceResult {
    output: String,
    node_label: String,
    model_name: Option<String>,
    job_id: Option<String>,
    status: Option<String>,
    error: Option<String>,
    runtime_metrics: Option<RuntimeMetrics>,
    job_payload: serde_json::Value,
    routing: String,
    local_fallback_code: Option<String>,
    local_fallback_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalAgentInferenceResponse {
    request_id: String,
    routing: String,
    node_id: String,
    model: Option<String>,
    output: String,
    status: String,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalAttemptFailure {
    code: String,
    message: String,
    network_fallback: bool,
}

impl LocalAttemptFailure {
    fn new(code: impl Into<String>, message: impl Into<String>, network_fallback: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            network_fallback,
        }
    }

    fn user_message(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

fn local_agent_url(path: &str) -> String {
    let base = std::env::var("OPENGPU_LOCAL_AGENT_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| {
            value.starts_with("http://127.0.0.1:") || value.starts_with("http://[::1]:")
        })
        .unwrap_or_else(|| "http://127.0.0.1:11435".to_string());
    format!("{base}{path}")
}

fn local_agent_token() -> Result<String, String> {
    let path = config_dir().join("local-agent-token");
    let token = std::fs::read_to_string(&path)
        .map_err(|_| "local node agent is not running or has not created its token".to_string())?;
    let token = token.trim();
    if token.len() < 32 {
        return Err("local node agent token is invalid".to_string());
    }
    Ok(token.to_string())
}

fn local_agent_error(error: ureq::Error) -> LocalAttemptFailure {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            local_agent_failure_from_body(status, &body)
        }
        ureq::Error::Transport(error) => LocalAttemptFailure::new(
            "LOCAL_AGENT_UNAVAILABLE",
            format!("local node agent is unavailable: {error}"),
            true,
        ),
    }
}

fn local_agent_failure_from_body(status: u16, body: &str) -> LocalAttemptFailure {
    let value = serde_json::from_str::<serde_json::Value>(body).unwrap_or_default();
    let code = value["code"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("LOCAL_AGENT_REJECTED");
    let message = value["error"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("local node agent rejected the request");
    let network_fallback = match value["fallback"].as_str() {
        Some("network") => true,
        Some(_) => false,
        None => status != 400,
    };
    LocalAttemptFailure::new(code, message, network_fallback)
}

fn no_execution_route_error(local: &LocalAttemptFailure, network: &str) -> String {
    format!(
        "no permitted execution route remains; local [{}]: {}; network: {}",
        local.code, local.message, network
    )
}

fn should_try_local(routing: RequestRoutingMode, execution_mode: ExecutionMode) -> bool {
    routing != RequestRoutingMode::NetworkOnly && execution_mode == ExecutionMode::Single
}

fn may_fallback_to_network(routing: RequestRoutingMode, failure: &LocalAttemptFailure) -> bool {
    routing == RequestRoutingMode::LocalFirst && failure.network_fallback
}

fn network_route_is_unreachable(error: &str) -> bool {
    error.starts_with("network routing failed: request failed:")
        || (error.starts_with("remote job ")
            && error.contains(" did not complete: request failed:"))
}

fn should_retry_local_offline(
    routing: RequestRoutingMode,
    failure: &LocalAttemptFailure,
    network_error: &str,
) -> bool {
    routing == RequestRoutingMode::LocalFirst
        && failure.code == "LOCAL_MODEL_UNSUITABLE"
        && network_route_is_unreachable(network_error)
}

fn run_inference_via_local_agent(
    prompt: &str,
    model: Option<&str>,
    backend: Backend,
    max_tokens: u32,
    timeout_secs: u64,
    force_local: bool,
) -> Result<InferenceResult, LocalAttemptFailure> {
    let token = local_agent_token()
        .map_err(|message| LocalAttemptFailure::new("LOCAL_AGENT_UNAVAILABLE", message, true))?;
    let request_id = format!("local-{}", uuid::Uuid::new_v4().simple());
    let payload = serde_json::json!({
        "request_id": request_id,
        "prompt": prompt,
        "model": model,
        "backend": backend.as_str(),
        "max_tokens": max_tokens,
        "force_local": force_local,
    });
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .post(&local_agent_url("/local/v1/chat/completions"))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(local_agent_error)?;
    let response: LocalAgentInferenceResponse = response.into_json().map_err(|error| {
        LocalAttemptFailure::new(
            "LOCAL_RESPONSE_INVALID",
            format!("invalid local node response: {error}"),
            true,
        )
    })?;
    let job_payload = serde_json::json!({
        "request_id": response.request_id,
        "assigned_node_id": response.node_id,
        "status": response.status,
        "model": response.model,
        "routing": response.routing,
        "output": response.output,
        "error": response.error,
    });
    Ok(InferenceResult {
        output: response.output,
        node_label: format!("local/{}", response.node_id),
        model_name: response.model,
        job_id: None,
        status: Some(response.status),
        error: response.error,
        runtime_metrics: None,
        job_payload,
        routing: response.routing,
        local_fallback_code: None,
        local_fallback_reason: None,
    })
}

fn run_inference(
    config: &Config,
    prompt: &str,
    model: Option<&str>,
    backend: Backend,
    max_tokens: u32,
    max_tokens_source: &str,
    execution_mode: ExecutionMode,
    routing: RequestRoutingMode,
    timeout_secs: u64,
    interval_secs: u64,
) -> Result<InferenceResult, String> {
    let local_eligible = should_try_local(routing, execution_mode);
    let local_result = if local_eligible {
        Some(run_inference_via_local_agent(
            prompt,
            model,
            backend,
            max_tokens,
            timeout_secs,
            routing == RequestRoutingMode::LocalOnly,
        ))
    } else if routing != RequestRoutingMode::NetworkOnly {
        Some(Err(LocalAttemptFailure::new(
            "LOCAL_MODE_UNSUPPORTED",
            "decomposed execution requires the control plane",
            true,
        )))
    } else {
        None
    };
    if let Some(Ok(result)) = local_result {
        return Ok(result);
    }
    let fallback = local_result.and_then(Result::err);
    if routing == RequestRoutingMode::LocalOnly {
        return Err(fallback.map_or_else(
            || "LOCAL_EXECUTION_UNAVAILABLE: local execution is unavailable".to_string(),
            |failure| failure.user_message(),
        ));
    }
    if fallback
        .as_ref()
        .is_some_and(|failure| !may_fallback_to_network(routing, failure))
    {
        return Err(fallback.expect("checked local failure").user_message());
    }
    let network_result = run_inference_via_control_plane(
        config,
        prompt,
        model,
        backend,
        max_tokens,
        max_tokens_source,
        execution_mode,
        timeout_secs,
        interval_secs,
    );
    let mut result = match network_result {
        Ok(result) => result,
        Err(network_error) => {
            if let Some(local) = fallback.as_ref() {
                if should_retry_local_offline(routing, local, &network_error) {
                    match run_inference_via_local_agent(
                        prompt,
                        model,
                        backend,
                        max_tokens,
                        timeout_secs,
                        true,
                    ) {
                        Ok(mut result) => {
                            result.routing = "local-offline".to_string();
                            result.local_fallback_code = Some(local.code.clone());
                            result.local_fallback_reason = Some(local.message.clone());
                            return Ok(result);
                        }
                        Err(retry_error) => {
                            return Err(format!(
                                "{}; offline local retry [{}]: {}",
                                no_execution_route_error(local, &network_error),
                                retry_error.code,
                                retry_error.message
                            ));
                        }
                    }
                }
            }
            return Err(match fallback.as_ref() {
                Some(local) => no_execution_route_error(local, &network_error),
                None => network_error,
            });
        }
    };
    result.local_fallback_code = fallback.as_ref().map(|failure| failure.code.clone());
    result.local_fallback_reason = fallback.map(|failure| failure.message);
    Ok(result)
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
struct RuntimeMetrics {
    total_duration_ms: Option<f64>,
    load_duration_ms: Option<f64>,
    prompt_eval_count: Option<u64>,
    prompt_eval_duration_ms: Option<f64>,
    prompt_eval_rate: Option<f64>,
    eval_count: Option<u64>,
    eval_duration_ms: Option<f64>,
    eval_rate: Option<f64>,
}

#[derive(serde::Deserialize)]
struct LocalModelManifestRecord {
    name: String,
    active: bool,
    source_path: Option<String>,
}

/// Submit through the control plane and wait for the scheduler result.
fn run_inference_via_control_plane(
    config: &Config,
    prompt: &str,
    model: Option<&str>,
    backend: crate::types::Backend,
    max_tokens: u32,
    max_tokens_source: &str,
    execution_mode: ExecutionMode,
    timeout_secs: u64,
    interval_secs: u64,
) -> Result<InferenceResult, String> {
    let (request_id, job) = build_job_submission_payload(
        prompt,
        model,
        backend,
        max_tokens,
        max_tokens_source,
        execution_mode,
    );
    match http_post_json(&config.control_plane_url, "/v1/jobs", &job) {
        Ok(record) => {
            let job_id = record["job_id"].as_str().unwrap_or(&request_id).to_string();
            let completed = wait_for_job(config, &job_id, timeout_secs, interval_secs)
                .map_err(|error| format!("remote job {job_id} did not complete: {error}"))?;
            let status = job_state(&completed);
            if matches!(status.as_str(), "failed" | "cancelled" | "canceled") {
                let error = completed
                    .get("error")
                    .and_then(|value| value.as_str())
                    .unwrap_or("remote job reached a failed terminal state");
                return Err(format!("remote job {job_id} {status}: {error}"));
            }
            let output = remote_job_output(&completed)
                .ok_or_else(|| format!("remote job {job_id} completed without output"))?;
            let runtime_metrics = runtime_metrics_from_payload(&completed)
                .or_else(|| runtime_metrics_from_output(&output));
            Ok(InferenceResult {
                output,
                node_label: completed
                    .get("assigned_node_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("control-plane")
                    .to_string(),
                model_name: model.map(|m| m.to_string()),
                job_id: Some(job_id),
                status: Some(status),
                error: completed
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                runtime_metrics,
                job_payload: completed,
                routing: "network".to_string(),
                local_fallback_code: None,
                local_fallback_reason: None,
            })
        }
        Err(error) => Err(format!("network routing failed: {error}")),
    }
}

fn build_job_submission_payload(
    prompt: &str,
    model: Option<&str>,
    backend: Backend,
    max_tokens: u32,
    max_tokens_source: &str,
    execution_mode: ExecutionMode,
) -> (String, serde_json::Value) {
    let request_id = format!("req-{}", uuid::Uuid::new_v4().simple());
    let job = serde_json::json!({
        "request_id": request_id,
        "prompt": prompt,
        "preferred_backend": backend.as_str(),
        "execution_mode": execution_mode.as_str(),
        "model": model,
        "max_tokens": max_tokens,
        "max_tokens_source": max_tokens_source,
    });
    (request_id, job)
}

fn default_max_tokens_for_prompt(prompt: &str) -> u32 {
    let trimmed = prompt.trim();
    let lower = trimmed.to_ascii_lowercase();

    if looks_like_short_computation(trimmed) {
        return 4;
    }

    if lower.contains("one word")
        || lower.contains("one number")
        || lower.contains("answer only")
        || lower.contains("final number")
    {
        return 16;
    }

    if looks_like_complete_code_prompt(&lower) {
        return 2048;
    }

    if looks_like_long_form_prompt(&lower) {
        return 1536;
    }

    512
}

fn looks_like_complete_code_prompt(lower_prompt: &str) -> bool {
    let direct_markers = [
        "complete program",
        "complete source",
        "complete code",
        "full program",
        "full source",
        "entire program",
        "working program",
        "turbo c program",
    ];

    if direct_markers
        .iter()
        .any(|marker| lower_prompt.contains(marker))
    {
        return true;
    }

    let program_request = ["write a program", "create a program", "make a program"]
        .iter()
        .any(|marker| lower_prompt.contains(marker));
    let code_context = [
        "c program",
        "turbo c",
        "source",
        "code",
        "binary file",
        "file handling",
    ]
    .iter()
    .any(|marker| lower_prompt.contains(marker));

    program_request && code_context
}

fn looks_like_long_form_prompt(lower_prompt: &str) -> bool {
    let markers = [
        "detailed history",
        "history of",
        "from its origins to today",
        "comprehensive",
        "in detail",
        "detailed",
        "full history",
        "deep dive",
        "report",
        "overview",
        "timeline",
        "write an article",
        "write a history",
        "explain the history",
    ];

    markers.iter().any(|marker| lower_prompt.contains(marker))
}

fn looks_like_short_computation(prompt: &str) -> bool {
    let compact: String = prompt.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.len() > 80 {
        return false;
    }

    let has_digit = compact.chars().any(|ch| ch.is_ascii_digit());
    let has_operator = compact
        .chars()
        .any(|ch| matches!(ch, '+' | '-' | '*' | '/' | '=' | '×' | '÷'));
    if !has_digit || !has_operator {
        return false;
    }

    compact
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "+-*/=().?:,'\"×÷".contains(ch))
}

fn effective_max_tokens(prompt: &str, max_tokens: Option<u32>) -> u32 {
    max_tokens
        .unwrap_or_else(|| default_max_tokens_for_prompt(prompt))
        .max(1)
}

fn max_tokens_source(max_tokens: Option<u32>) -> &'static str {
    if max_tokens.is_some() {
        "explicit"
    } else {
        "auto"
    }
}

fn effective_execution_mode(
    decompose: bool,
    execution_mode: Option<ExecutionMode>,
) -> ExecutionMode {
    if decompose {
        ExecutionMode::Decompose
    } else {
        execution_mode.unwrap_or(ExecutionMode::Single)
    }
}

fn submit_job(
    config: &Config,
    prompt: &str,
    model: Option<&str>,
    backend: Backend,
    max_tokens: u32,
    max_tokens_source: &str,
    execution_mode: ExecutionMode,
) -> Result<serde_json::Value, String> {
    let (_, job) = build_job_submission_payload(
        prompt,
        model,
        backend,
        max_tokens,
        max_tokens_source,
        execution_mode,
    );
    http_post_json(&config.control_plane_url, "/v1/jobs", &job)
}

fn job_status_path(job_id: &str) -> Result<String, String> {
    let trimmed = job_id.trim();
    if trimmed.is_empty() {
        return Err("job id cannot be empty".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err("job id may only contain letters, numbers, hyphen, or underscore".to_string());
    }
    Ok(format!("/v1/jobs/{trimmed}"))
}

fn get_job(config: &Config, job_id: &str) -> Result<serde_json::Value, String> {
    let path = job_status_path(job_id)?;
    operator_get_json(
        &config.control_plane_url,
        &path,
        auth_token::effective_operator_token(config).as_deref(),
    )
}

fn job_state(payload: &serde_json::Value) -> String {
    payload
        .get("status")
        .or_else(|| payload.get("state"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

fn job_is_terminal(payload: &serde_json::Value) -> bool {
    matches!(
        job_state(payload).as_str(),
        "completed" | "failed" | "cancelled" | "canceled"
    )
}

fn job_plan_payload(payload: &serde_json::Value) -> &serde_json::Value {
    payload.get("job").unwrap_or(payload)
}

fn graph_nodes(payload: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    job_plan_payload(payload)
        .pointer("/graph/nodes")
        .and_then(|value| value.as_array())
}

fn graph_progress_counts(payload: &serde_json::Value) -> Option<(usize, usize, usize)> {
    let nodes = graph_nodes(payload)?;
    if nodes.is_empty() {
        return None;
    }

    let completed = nodes
        .iter()
        .filter(|node| {
            node.get("status")
                .and_then(|value| value.as_str())
                .map(|status| status.eq_ignore_ascii_case("completed"))
                .unwrap_or(false)
        })
        .count();
    let running = nodes
        .iter()
        .filter(|node| {
            node.get("status")
                .and_then(|value| value.as_str())
                .map(|status| status.eq_ignore_ascii_case("running"))
                .unwrap_or(false)
        })
        .count();

    Some((completed, running, nodes.len()))
}

fn active_graph_node_name(payload: &serde_json::Value) -> Option<String> {
    let job = job_plan_payload(payload);
    let active_id = job
        .get("active_graph_node_id")
        .and_then(|value| value.as_str())?;

    graph_nodes(payload)
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|node| node.get("id").and_then(|value| value.as_str()) == Some(active_id))
        })
        .and_then(|node| {
            node.get("name")
                .and_then(|value| value.as_str())
                .or_else(|| node.get("id").and_then(|value| value.as_str()))
        })
        .map(str::to_string)
        .or_else(|| Some(active_id.to_string()))
}

fn active_graph_node(payload: &serde_json::Value) -> Option<&serde_json::Value> {
    let job = job_plan_payload(payload);
    let active_id = job
        .get("active_graph_node_id")
        .and_then(|value| value.as_str())?;

    graph_nodes(payload).and_then(|nodes| {
        nodes
            .iter()
            .find(|node| node.get("id").and_then(|value| value.as_str()) == Some(active_id))
    })
}

fn active_graph_action_label(payload: &serde_json::Value) -> &'static str {
    active_graph_node(payload)
        .map(|node| {
            if graph_node_is_reducer(node) {
                "merging"
            } else {
                "processing"
            }
        })
        .unwrap_or("waiting")
}

fn job_degradation_message(payload: &serde_json::Value) -> Option<&str> {
    job_plan_payload(payload)
        .pointer("/degradation/message")
        .and_then(|value| value.as_str())
        .or_else(|| {
            payload
                .pointer("/degradation/message")
                .and_then(|value| value.as_str())
        })
}

fn graph_node_label(node: &serde_json::Value) -> String {
    node.get("name")
        .and_then(|value| value.as_str())
        .or_else(|| node.get("id").and_then(|value| value.as_str()))
        .unwrap_or("unknown chunk")
        .to_string()
}

fn running_graph_chunk_summaries(payload: &serde_json::Value) -> Vec<String> {
    let Some(nodes) = graph_nodes(payload) else {
        return Vec::new();
    };

    nodes
        .iter()
        .filter(|node| {
            node.get("status")
                .and_then(|value| value.as_str())
                .map(|status| status.eq_ignore_ascii_case("running"))
                .unwrap_or(false)
        })
        .map(|node| {
            let mut summary = graph_node_label(node);
            if let Some(assigned_node) = node
                .get("assigned_node_id")
                .and_then(|value| value.as_str())
            {
                summary.push_str(&format!("@{assigned_node}"));
            }
            if let Some(worker) = node.get("worker_id").and_then(|value| value.as_str()) {
                summary.push_str(&format!("/worker={worker}"));
            } else {
                summary.push_str("/worker=pending");
            }
            summary
        })
        .collect()
}

fn job_wait_progress_signature(payload: &serde_json::Value) -> Option<String> {
    let job = job_plan_payload(payload);
    let graph_enabled = job
        .get("graph_execution_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !graph_enabled {
        return None;
    }

    let (completed, running, total) = graph_progress_counts(payload)?;
    let active =
        active_graph_node_name(payload).unwrap_or_else(|| "waiting for next chunk".to_string());
    let action = active_graph_action_label(payload);
    let running_chunks = running_graph_chunk_summaries(payload).join(";");
    Some(format!(
        "{}|{completed}|{running}|{total}|{action}|{active}|{running_chunks}",
        job_state(payload)
    ))
}

fn print_job_wait_progress(payload: &serde_json::Value) {
    if let Some(message) = job_degradation_message(payload) {
        eprintln!("job degradation: {message}");
    }
    if let Some((completed, running, total)) = graph_progress_counts(payload) {
        let active =
            active_graph_node_name(payload).unwrap_or_else(|| "waiting for next chunk".to_string());
        let action = active_graph_action_label(payload);
        let running_chunks = running_graph_chunk_summaries(payload);
        if running_chunks.len() > 1 {
            eprintln!(
                "job progress: status={} chunks={completed}/{total} done, {running} running, {action}={active}, runningChunks={}",
                theme::status(&job_state(payload)),
                running_chunks.join("; "),
            );
        } else {
            eprintln!(
                "job progress: status={} chunks={completed}/{total} done, {running} running, {action}={active}",
                theme::status(&job_state(payload)),
            );
        }
    }
}

struct CliSpinner {
    enabled: bool,
    frame: usize,
    width: usize,
}

impl CliSpinner {
    fn new() -> Self {
        Self {
            enabled: io::stderr().is_terminal(),
            frame: 0,
            width: 0,
        }
    }

    fn tick(&mut self, label: &str) {
        if !self.enabled {
            return;
        }

        const FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
        let line = format!("{} {}", FRAMES[self.frame % FRAMES.len()], label);
        self.frame = self.frame.wrapping_add(1);
        self.width = self.width.max(line.chars().count());
        eprint!("\r{line:<width$}", width = self.width);
        let _ = io::stderr().flush();
    }

    fn clear(&mut self) {
        if !self.enabled || self.width == 0 {
            return;
        }

        eprint!("\r{:<width$}\r", "", width = self.width);
        let _ = io::stderr().flush();
        self.width = 0;
    }
}

fn job_wait_spinner_label(payload: &serde_json::Value) -> String {
    let status = job_state(payload);
    if let Some(message) = job_degradation_message(payload) {
        return format!("waiting for job: status={status}, {message}");
    }
    if let Some((completed, running, total)) = graph_progress_counts(payload) {
        let active =
            active_graph_node_name(payload).unwrap_or_else(|| "waiting for next chunk".to_string());
        let action = active_graph_action_label(payload);
        format!(
            "waiting for job: status={} chunks={completed}/{total} done, {running} running, {action}={active}",
            status
        )
    } else {
        format!("waiting for job: status={status}")
    }
}

fn graph_node_dependency_suffix(status: &str, blocked_by: &[&str]) -> String {
    if blocked_by.is_empty() {
        return String::new();
    }

    let dependencies = blocked_by.join(", ");
    if status.eq_ignore_ascii_case("waiting") {
        format!(" waiting for {dependencies}")
    } else {
        format!(" blocked by {dependencies}")
    }
}

fn graph_node_is_reducer(node: &serde_json::Value) -> bool {
    let responsibility = node
        .get("responsibility")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let name = node
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let id = node
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    responsibility.contains("merge")
        || name.contains("final synthesis")
        || name.contains("final answer")
        || id.contains("final")
        || id.contains("merge")
}

fn graph_node_display_status(node: &serde_json::Value) -> &'static str {
    let status = node
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");

    match status.to_ascii_lowercase().as_str() {
        "completed" => "done",
        "running" if graph_node_is_reducer(node) => "merging",
        "running" => "processing",
        "failed" => "failed",
        "waiting" => "waiting",
        "ready" => "ready",
        _ => "unknown",
    }
}

fn print_job_plan_progress(payload: &serde_json::Value) {
    let job = job_plan_payload(payload);
    let graph_enabled = job
        .get("graph_execution_enabled")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let Some(nodes) = graph_nodes(payload) else {
        return;
    };
    if nodes.is_empty() {
        return;
    }

    let strategy = job
        .pointer("/plan/strategy")
        .and_then(|value| value.as_str())
        .unwrap_or("unplanned");
    let summary = job
        .pointer("/plan/summary")
        .and_then(|value| value.as_str())
        .unwrap_or("No planner summary recorded.");
    let mode = job
        .get("execution_mode")
        .and_then(|value| value.as_str())
        .unwrap_or("single");
    let (completed, running, total) = graph_progress_counts(payload).unwrap_or((0, 0, nodes.len()));

    println!();
    theme::section("Job plan");
    theme::field("executionMode", mode);
    theme::field("strategy", strategy);
    theme::field(
        "graphExecution",
        if graph_enabled { "enabled" } else { "advisory" },
    );
    if graph_enabled {
        theme::field(
            "progress",
            format!("{completed}/{total} completed, {running} running"),
        );
    } else {
        theme::field("progress", "not chunked; plan recorded only");
    }
    println!("{summary}");

    for (index, node) in nodes.iter().enumerate() {
        let name = node
            .get("name")
            .and_then(|value| value.as_str())
            .or_else(|| node.get("id").and_then(|value| value.as_str()))
            .unwrap_or("planned job");
        let status = node
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let blocked_by = node
            .get("blocked_by")
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let suffix = graph_node_dependency_suffix(status, &blocked_by);
        println!(
            "  {}. [{}] {}{}",
            index + 1,
            theme::status(graph_node_display_status(node)),
            name,
            suffix
        );
    }
}

fn remote_job_output(payload: &serde_json::Value) -> Option<String> {
    fn output_from(value: &serde_json::Value) -> Option<String> {
        ["output", "final_output", "result"]
            .iter()
            .filter_map(|field| value.get(*field))
            .find_map(|value| {
                value.as_str().map(|text| text.to_string()).or_else(|| {
                    if value.is_null() {
                        None
                    } else {
                        Some(value.to_string())
                    }
                })
            })
    }

    output_from(payload)
        .or_else(|| payload.get("job").and_then(output_from))
        .or_else(|| payload.pointer("/job/graph").and_then(output_from))
}

fn runtime_metrics_from_payload(payload: &serde_json::Value) -> Option<RuntimeMetrics> {
    runtime_metrics_from_value(payload.get("runtime_metrics"))
        .or_else(|| runtime_metrics_from_value(payload.pointer("/job/runtime_metrics")))
        .or_else(|| {
            payload
                .pointer("/job/graph/nodes")
                .and_then(|value| value.as_array())
                .and_then(|nodes| runtime_metrics_from_graph_nodes(nodes))
        })
}

fn runtime_metrics_from_graph_nodes(nodes: &[serde_json::Value]) -> Option<RuntimeMetrics> {
    let metrics: Vec<RuntimeMetrics> = nodes
        .iter()
        .filter_map(|node| {
            runtime_metrics_from_value(node.get("runtime_metrics")).or_else(|| {
                node.get("output")
                    .and_then(|value| value.as_str())
                    .and_then(runtime_metrics_from_output)
            })
        })
        .collect();
    if metrics.is_empty() {
        return None;
    }

    Some(RuntimeMetrics {
        total_duration_ms: sum_metric(&metrics, |metric| metric.total_duration_ms),
        load_duration_ms: sum_metric(&metrics, |metric| metric.load_duration_ms),
        prompt_eval_count: sum_metric_u64(&metrics, |metric| metric.prompt_eval_count),
        prompt_eval_duration_ms: sum_metric(&metrics, |metric| metric.prompt_eval_duration_ms),
        prompt_eval_rate: average_metric(&metrics, |metric| metric.prompt_eval_rate),
        eval_count: sum_metric_u64(&metrics, |metric| metric.eval_count),
        eval_duration_ms: sum_metric(&metrics, |metric| metric.eval_duration_ms),
        eval_rate: average_metric(&metrics, |metric| metric.eval_rate),
    })
}

fn runtime_metrics_from_value(value: Option<&serde_json::Value>) -> Option<RuntimeMetrics> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    serde_json::from_value::<RuntimeMetrics>(value.clone())
        .ok()
        .filter(|metrics| !runtime_metrics_empty(metrics))
}

fn runtime_metrics_from_output(output: &str) -> Option<RuntimeMetrics> {
    let marker = "runtime_metrics=";
    let start = output.find(marker)? + marker.len();
    let rest = &output[start..];
    let json = extract_balanced_json(rest)?;
    serde_json::from_str::<RuntimeMetrics>(json)
        .ok()
        .filter(|metrics| !runtime_metrics_empty(metrics))
}

fn extract_balanced_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    for (offset, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&text[start..start + offset + ch.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

fn runtime_metrics_empty(metrics: &RuntimeMetrics) -> bool {
    metrics.total_duration_ms.is_none()
        && metrics.load_duration_ms.is_none()
        && metrics.prompt_eval_count.is_none()
        && metrics.prompt_eval_duration_ms.is_none()
        && metrics.prompt_eval_rate.is_none()
        && metrics.eval_count.is_none()
        && metrics.eval_duration_ms.is_none()
        && metrics.eval_rate.is_none()
}

fn sum_metric(
    metrics: &[RuntimeMetrics],
    getter: fn(&RuntimeMetrics) -> Option<f64>,
) -> Option<f64> {
    let values: Vec<f64> = metrics.iter().filter_map(getter).collect();
    (!values.is_empty()).then(|| values.iter().sum())
}

fn sum_metric_u64(
    metrics: &[RuntimeMetrics],
    getter: fn(&RuntimeMetrics) -> Option<u64>,
) -> Option<u64> {
    let values: Vec<u64> = metrics.iter().filter_map(getter).collect();
    (!values.is_empty()).then(|| values.iter().sum())
}

fn average_metric(
    metrics: &[RuntimeMetrics],
    getter: fn(&RuntimeMetrics) -> Option<f64>,
) -> Option<f64> {
    let values: Vec<f64> = metrics.iter().filter_map(getter).collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn print_runtime_metrics(metrics: Option<&RuntimeMetrics>) {
    let Some(metrics) = metrics else {
        return;
    };
    if runtime_metrics_empty(metrics) {
        return;
    }
    theme::section("Runtime metrics");
    if let Some(value) = metrics.total_duration_ms {
        theme::field("totalDurationMs", format_duration_ms(value));
    }
    if let Some(value) = metrics.load_duration_ms {
        theme::field("loadDurationMs", format_duration_ms(value));
    }
    if let Some(value) = metrics.prompt_eval_count {
        theme::field("promptEvalCountTokens", value.to_string());
    }
    if let Some(value) = metrics.prompt_eval_duration_ms {
        theme::field("promptEvalDurationMs", format_duration_ms(value));
    }
    if let Some(value) = metrics.prompt_eval_rate {
        theme::field(
            "promptEvalRateTokensPerSecond",
            format!("{value:.2} tokens/s"),
        );
    }
    if let Some(value) = metrics.eval_count {
        theme::field("evalCountTokens", value.to_string());
    }
    if let Some(value) = metrics.eval_duration_ms {
        theme::field("evalDurationMs", format_duration_ms(value));
    }
    if let Some(value) = metrics.eval_rate {
        theme::field("evalRateTokensPerSecond", format!("{value:.2} tokens/s"));
    }
}

fn format_duration_ms(value: f64) -> String {
    if value >= 1000.0 {
        format!("{:.2}s", value / 1000.0)
    } else {
        format!("{value:.0}ms")
    }
}

fn print_inference_output(output: &str) {
    if let Some((fields, response)) = parse_worker_output(output) {
        theme::section("Worker output");
        for (label, value) in fields {
            theme::field(&label, value);
        }
        if !response.trim().is_empty() {
            println!();
            println!("{}", response.trim());
        }
        return;
    }

    println!("{output}");
}

fn parse_worker_output(output: &str) -> Option<(Vec<(String, String)>, String)> {
    let text = output.trim();
    let metadata = text
        .strip_prefix("llama.cpp ")
        .or_else(|| text.strip_prefix("mlx-lm "))?;
    let response_marker = "; response=";
    let (metadata, response) = metadata
        .split_once(response_marker)
        .unwrap_or((metadata, ""));
    let fields = metadata
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            let key = key.trim();
            if key == "runtime_metrics" {
                return None;
            }
            Some((camel_case_label(key), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    (!fields.is_empty()).then(|| (fields, response.trim().to_string()))
}

fn camel_case_label(value: &str) -> String {
    let mut words = value.split('_').filter(|part| !part.is_empty());
    let Some(first) = words.next() else {
        return value.to_string();
    };
    let mut label = first.to_string();
    for word in words {
        let mut chars = word.chars();
        if let Some(first_char) = chars.next() {
            label.push(first_char.to_ascii_uppercase());
            label.extend(chars);
        }
    }
    label
}

fn wait_for_job(
    config: &Config,
    job_id: &str,
    timeout_secs: u64,
    interval_secs: u64,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let interval = Duration::from_secs(interval_secs.max(1));
    let mut last_progress_signature: Option<String> = None;
    let mut spinner = CliSpinner::new();

    loop {
        let payload = get_job(config, job_id)?;
        if job_is_terminal(&payload) {
            spinner.clear();
            return Ok(payload);
        }
        let progress_signature = job_wait_progress_signature(&payload);
        if progress_signature.is_some() && progress_signature != last_progress_signature {
            spinner.clear();
            print_job_wait_progress(&payload);
            last_progress_signature = progress_signature;
        }
        spinner.tick(&job_wait_spinner_label(&payload));
        if Instant::now() >= deadline {
            spinner.clear();
            let detail = timeout_job_detail(&payload);
            return Err(format!(
                "timed out waiting for job {job_id} while status was {}{detail}",
                job_state(&payload),
            ));
        }
        thread::sleep(interval);
    }
}

fn timeout_job_detail(payload: &serde_json::Value) -> String {
    let job = job_plan_payload(payload);
    let mut details = Vec::new();

    if let Some(node_id) = job.get("assigned_node_id").and_then(|value| value.as_str()) {
        details.push(format!("assignedNode={node_id}"));
    }

    if let Some(active_node_id) = job
        .get("active_graph_node_id")
        .and_then(|value| value.as_str())
    {
        details.push(format!("activeChunk={active_node_id}"));
    }

    let running_chunks = running_graph_chunk_summaries(payload);
    if !running_chunks.is_empty() {
        details.push(format!("runningChunks={}", running_chunks.join("; ")));
    }

    if details.is_empty() {
        String::new()
    } else {
        format!(" ({})", details.join(", "))
    }
}

fn print_job_response(payload: &serde_json::Value, json: bool) -> Result<(), String> {
    if json {
        return print_json(payload);
    }

    if let Some(job_id) = payload.get("job_id").and_then(|value| value.as_str()) {
        theme::field("jobId", job_id);
    }
    if let Some(request_id) = payload.get("request_id").and_then(|value| value.as_str()) {
        theme::field("requestId", request_id);
    }
    theme::field("status", theme::status(&job_state(payload)));
    if let Some(output) = payload.get("output").and_then(|value| value.as_str()) {
        theme::field("output", output);
    }
    if let Some(error) = payload.get("error").and_then(|value| value.as_str()) {
        theme::field("error", error);
    }
    print_job_plan_progress(payload);
    Ok(())
}

fn resolve_local_model_path(
    model_dir: &std::path::Path,
    model_name: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    if let Some(path) = resolve_manifest_model_path(model_dir, model_name) {
        return Ok(path);
    }

    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(name) = model_name {
        search_dirs.push(model_dir.join(sanitize_for_path(name)));
    }
    search_dirs.push(model_dir.to_path_buf());

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("gguf") {
                    files.push(path);
                }
            }
        }
        if !files.is_empty() {
            break;
        }
    }

    files.sort();
    files
        .into_iter()
        .next()
        .ok_or_else(|| format!("no .gguf file found in {}", model_dir.display()))
}

fn resolve_manifest_model_path(
    model_dir: &std::path::Path,
    model_name: Option<&str>,
) -> Option<std::path::PathBuf> {
    let manifest_dir = model_dir.join(".opengpu");
    let entries = std::fs::read_dir(manifest_dir).ok()?;
    let mut fallback = None;

    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let raw = std::fs::read_to_string(entry.path()).ok()?;
            let record = serde_json::from_str::<LocalModelManifestRecord>(&raw).ok()?;
            let matches_name = model_name
                .map(|name| record.name == name)
                .unwrap_or(record.active);
            if matches_name {
                let source_path = record.source_path.as_ref()?;
                let path = std::path::PathBuf::from(source_path);
                if path.is_file() {
                    return Some(path);
                }
            } else if record.active {
                fallback = record
                    .source_path
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .filter(|path| path.is_file());
            }
        }
    }

    fallback
}

fn sanitize_for_path(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn control_plane_endpoint(control_plane_url: &str, path: &str) -> Result<String, String> {
    let base = control_plane_url.trim().trim_end_matches('/');
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err("control-plane-url must start with http:// or https://".to_string());
    }
    if !path.starts_with('/') {
        return Err("control-plane API path must start with /".to_string());
    }
    Ok(format!("{base}{path}"))
}

fn read_control_plane_response(response: ureq::Response) -> Result<serde_json::Value, String> {
    let body = response
        .into_string()
        .map_err(|error| format!("read failed: {error}"))?;
    serde_json::from_str(&body).map_err(|error| error.to_string())
}

fn control_plane_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            if body.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                format!("HTTP {status}: {}", body.trim())
            }
        }
        ureq::Error::Transport(error) => format!("request failed: {error}"),
    }
}

/// HTTP POST a JSON payload to an http:// or https:// control-plane URL.
fn http_post_json(
    control_plane_url: &str,
    path: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use std::time::Duration;

    let endpoint = control_plane_endpoint(control_plane_url, path)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let response = agent
        .post(&endpoint)
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(control_plane_error)?;
    read_control_plane_response(response)
}
fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    serde_json::to_string_pretty(value)
        .map(|output| {
            println!("{output}");
        })
        .map_err(|error| error.to_string())
}

const CREDIT_LOG_WINDOW_MILLIS: u64 = 30 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LocalCreditTotals {
    node_id: String,
    remaining_credits: f64,
    earned_credits: f64,
    used_credits: f64,
    ledger_entries: usize,
}

#[derive(Clone, Debug)]
struct CreditLogSync {
    current_log_path: Option<PathBuf>,
    current_entries: Vec<serde_json::Value>,
    appended_entries: usize,
    page: usize,
    limit: usize,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn current_node_id(config: &Config) -> String {
    load_identity()
        .ok()
        .flatten()
        .map(|identity| device_id_for_identity(&identity))
        .unwrap_or_else(|| config.device_id.clone())
}

fn credit_log_timestamp(path: &std::path::Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("data_")?
        .strip_suffix("_credit.log")?
        .parse::<u64>()
        .ok()
}

fn credit_log_files() -> Vec<PathBuf> {
    let mut files = std::fs::read_dir(config::config_dir())
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| credit_log_timestamp(path).is_some())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| credit_log_timestamp(path).unwrap_or(0));
    files
}

fn credit_entry_key(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            Some(format!(
                "{}|{}|{}|{}",
                entry
                    .get("job_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                entry
                    .get("device_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                entry
                    .get("created_at")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
                entry
                    .get("amount")
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            ))
        })
}

fn logged_credit_key(log_entry: &serde_json::Value) -> Option<String> {
    log_entry
        .get("ledger")
        .and_then(credit_entry_key)
        .or_else(|| credit_entry_key(log_entry))
}

fn read_credit_log_entries(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .map(|raw| {
            raw.lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn known_logged_credit_keys(files: &[PathBuf]) -> std::collections::BTreeSet<String> {
    files
        .iter()
        .flat_map(|path| read_credit_log_entries(path))
        .filter_map(|entry| logged_credit_key(&entry))
        .collect()
}

fn local_credit_entries<'a>(
    payload: &'a serde_json::Value,
    node_id: &str,
) -> Vec<&'a serde_json::Value> {
    payload
        .get("ledger")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("device_id").and_then(|value| value.as_str()) == Some(node_id))
        .collect()
}

fn round_credits(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn local_credit_totals(payload: &serde_json::Value, node_id: &str) -> LocalCreditTotals {
    let entries = local_credit_entries(payload, node_id);
    let earned_credits = entries
        .iter()
        .map(|entry| {
            entry
                .get("amount")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0)
        })
        .filter(|amount| *amount > 0.0)
        .sum::<f64>();
    let used_credits = entries
        .iter()
        .map(|entry| {
            entry
                .get("amount")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0)
        })
        .filter(|amount| *amount < 0.0)
        .map(f64::abs)
        .sum::<f64>();
    let remaining_credits = payload
        .get("by_node")
        .and_then(|value| value.get(node_id))
        .and_then(|value| value.as_f64())
        .unwrap_or(earned_credits - used_credits);

    LocalCreditTotals {
        node_id: node_id.to_string(),
        remaining_credits: round_credits(remaining_credits),
        earned_credits: round_credits(earned_credits),
        used_credits: round_credits(used_credits),
        ledger_entries: entries.len(),
    }
}

fn paged_current_credit_entries(
    entries: &[serde_json::Value],
    page: usize,
    limit: usize,
) -> Vec<serde_json::Value> {
    let limit = limit.max(1);
    entries
        .iter()
        .rev()
        .skip(page.saturating_mul(limit))
        .take(limit)
        .cloned()
        .collect()
}

fn sync_local_credit_log(
    payload: &serde_json::Value,
    node_id: &str,
    page: usize,
    limit: usize,
    now_ms: u64,
) -> Result<CreditLogSync, String> {
    let local_entries = local_credit_entries(payload, node_id);
    let existing_files = credit_log_files();
    let known_keys = known_logged_credit_keys(&existing_files);
    let new_entries = local_entries
        .into_iter()
        .filter(|entry| {
            credit_entry_key(entry)
                .map(|key| !known_keys.contains(&key))
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut current_log_path = existing_files.last().cloned();
    if !new_entries.is_empty() {
        let should_rotate = current_log_path
            .as_deref()
            .and_then(credit_log_timestamp)
            .map(|started_at| now_ms.saturating_sub(started_at) >= CREDIT_LOG_WINDOW_MILLIS)
            .unwrap_or(true);
        if should_rotate {
            current_log_path = Some(config::config_dir().join(format!("data_{now_ms}_credit.log")));
        }
        let path = current_log_path
            .as_ref()
            .ok_or_else(|| "credit log path unavailable".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create credit log directory: {error}"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("failed to open credit log `{}`: {error}", path.display()))?;
        for entry in &new_entries {
            let line = serde_json::json!({
                "synced_at_millis": now_ms,
                "node_id": node_id,
                "ledger": entry,
            });
            writeln!(
                file,
                "{}",
                serde_json::to_string(&line).map_err(|error| error.to_string())?
            )
            .map_err(|error| format!("failed to write credit log: {error}"))?;
        }
    }

    let current_entries = current_log_path
        .as_deref()
        .map(read_credit_log_entries)
        .unwrap_or_default();
    Ok(CreditLogSync {
        current_log_path,
        current_entries: paged_current_credit_entries(&current_entries, page, limit),
        appended_entries: new_entries.len(),
        page,
        limit: limit.max(1),
    })
}

fn local_credits_payload(
    payload: &serde_json::Value,
    node_id: &str,
    sync: &CreditLogSync,
) -> serde_json::Value {
    let totals = local_credit_totals(payload, node_id);
    serde_json::json!({
        "node_id": totals.node_id,
        "remaining_credits": totals.remaining_credits,
        "earned_credits": totals.earned_credits,
        "used_credits": totals.used_credits,
        "ledger_entries": totals.ledger_entries,
        "current_log": sync.current_log_path.as_ref().map(|path| path.display().to_string()),
        "current_log_entries": sync.current_entries,
        "appended_entries": sync.appended_entries,
        "page": sync.page,
        "limit": sync.limit,
    })
}

fn print_local_credits_report(payload: &serde_json::Value) {
    println!(
        "nodeId: {}",
        payload["node_id"].as_str().unwrap_or("unknown")
    );
    println!(
        "remainingCredits: {:.2}",
        payload["remaining_credits"].as_f64().unwrap_or(0.0)
    );
    println!(
        "earnedCredits: {:.2}",
        payload["earned_credits"].as_f64().unwrap_or(0.0)
    );
    println!(
        "usedCredits: {:.2}",
        payload["used_credits"].as_f64().unwrap_or(0.0)
    );
    println!(
        "currentLog: {}",
        payload["current_log"].as_str().unwrap_or("none")
    );
    println!(
        "logPage: {} limit {}",
        payload["page"].as_u64().unwrap_or(0),
        payload["limit"].as_u64().unwrap_or(0)
    );
    if let Some(entries) = payload["current_log_entries"].as_array() {
        if entries.is_empty() {
            println!("entries: none");
        } else {
            println!("entries:");
            for entry in entries {
                let ledger = entry.get("ledger").unwrap_or(entry);
                let amount = ledger
                    .get("amount")
                    .and_then(|value| value.as_f64())
                    .unwrap_or(0.0);
                let currency = ledger
                    .get("currency")
                    .and_then(|value| value.as_str())
                    .unwrap_or("credits");
                let job_id = ledger
                    .get("job_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown-job");
                let created_at = ledger
                    .get("created_at")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown-time");
                let graph_node_id = ledger
                    .get("metadata")
                    .and_then(|metadata| metadata.get("graph_node_id"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("job");
                println!("  {created_at} {amount:.2} {currency} {job_id} {graph_node_id}");
            }
        }
    }
}

/// Fetch a JSON resource from the control plane using an operator bearer token.
/// Supports both local http:// control planes and hosted https:// control planes.
fn operator_get_json(
    control_plane_url: &str,
    path: &str,
    auth_token: Option<&str>,
) -> Result<serde_json::Value, String> {
    use std::time::Duration;

    let endpoint = control_plane_endpoint(control_plane_url, path)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let mut request = agent.get(&endpoint);
    if let Some(token) = auth_token.filter(|token| !token.is_empty()) {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = request.call().map_err(control_plane_error)?;
    read_control_plane_response(response)
}
fn colored_state(
    value: bool,
    _active_color: Color,
    active_text: &str,
    inactive_text: &str,
) -> String {
    theme::boolean(value, active_text, inactive_text)
}

#[derive(Clone, Debug)]
struct PowerState {
    source: String,
    on_battery: bool,
    battery_percent: Option<u8>,
}

fn probe_power_state() -> PowerState {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("pmset").args(["-g", "batt"]).output() {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                let mut source = "unknown".to_string();
                let mut on_battery = true;
                let mut battery_percent = None;

                for line in stdout.lines() {
                    if line.starts_with("Now drawing from") {
                        source = line
                            .split_once('\'')
                            .map(|(_, rest)| rest.trim_matches('\'').to_string())
                            .unwrap_or_else(|| line.to_string());
                        on_battery = !source.to_lowercase().contains("ac power");
                    }

                    if let Some(percent_text) = line.split('%').next() {
                        if let Some(token) = percent_text
                            .split_whitespace()
                            .rev()
                            .find(|part| part.chars().all(|ch| ch.is_ascii_digit()))
                        {
                            battery_percent = token.parse::<u8>().ok();
                        }
                    }
                }

                return PowerState {
                    source,
                    on_battery,
                    battery_percent,
                };
            }
        }
    }

    PowerState {
        source: "unknown".to_string(),
        on_battery: false,
        battery_percent: None,
    }
}

fn policy_reason(
    config: &Config,
    power: &PowerState,
    active_model: Option<&str>,
    identity_ready: bool,
) -> Option<String> {
    if !identity_ready {
        return Some(
            "secure device identity is missing; `opengpu start` creates it automatically"
                .to_string(),
        );
    }

    if config.contribution_percent == 0 {
        return Some("contribution percent is unset".to_string());
    }

    if active_model.is_none() {
        return Some("no active model is selected".to_string());
    }

    if power.on_battery {
        if config.contribution_percent > 20 {
            return Some("battery power requires contribution percent <= 20".to_string());
        }

        if let Some(percent) = power.battery_percent {
            if percent <= 20 {
                return Some("battery level is too low to start work safely".to_string());
            }
        }
    }

    None
}

fn policy_allowed(
    config: &Config,
    power: &PowerState,
    active_model: Option<&str>,
    identity_ready: bool,
) -> bool {
    policy_reason(config, power, active_model, identity_ready).is_none()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalReadiness {
    ready_for_jobs: bool,
    readiness_reason: Option<String>,
    model_compatibility: Option<String>,
    model_compatibility_reason: Option<String>,
}

fn active_model_record(config: &Config, active_model: Option<&str>) -> Option<ModelRecord> {
    let models = list_models(config).ok()?;
    if let Some(active_model) = active_model {
        models
            .iter()
            .find(|model| model.name == active_model)
            .cloned()
            .or_else(|| models.into_iter().find(|model| model.active))
    } else {
        models.into_iter().find(|model| model.active)
    }
}

fn local_readiness(
    config: &Config,
    power: &PowerState,
    active_model: Option<&str>,
    identity_ready: bool,
) -> LocalReadiness {
    let policy_reason = policy_reason(config, power, active_model, identity_ready);
    let model = active_model_record(config, active_model);
    let model_compatibility = model.as_ref().and_then(|model| model.compatibility.clone());
    let model_compatibility_reason = model
        .as_ref()
        .and_then(|model| model.compatibility_reason.clone());
    let readiness_reason = if !config.connected {
        Some("node is disconnected".to_string())
    } else if config.paused {
        Some("node is paused".to_string())
    } else if let Some(reason) = policy_reason {
        Some(reason)
    } else if model_compatibility.as_deref() == Some("rejected") {
        model_compatibility_reason
            .clone()
            .or_else(|| Some("active model is not compatible with this node".to_string()))
    } else {
        None
    };

    LocalReadiness {
        ready_for_jobs: readiness_reason.is_none(),
        readiness_reason,
        model_compatibility,
        model_compatibility_reason,
    }
}

fn provider_count(
    config: &Config,
    power: &PowerState,
    active_model: Option<&str>,
    identity_ready: bool,
) -> usize {
    if local_readiness(config, power, active_model, identity_ready).ready_for_jobs {
        1
    } else {
        0
    }
}

fn latest_agent_state(config: &Config) -> Option<Heartbeat> {
    let path = config::config_dir().join("agent-state.json");
    let text = std::fs::read_to_string(path).ok()?;
    let heartbeat: Heartbeat = serde_json::from_str(&text).ok()?;
    if heartbeat.node_id == config.device_id {
        Some(heartbeat)
    } else {
        None
    }
}

fn live_readiness_from_agent(agent: &Heartbeat) -> Option<LocalReadiness> {
    let capabilities = agent.capabilities.as_ref()?;
    Some(LocalReadiness {
        ready_for_jobs: capabilities.ready_for_jobs,
        readiness_reason: capabilities
            .readiness_reason
            .clone()
            .or_else(|| agent.policy_reason.clone()),
        model_compatibility: None,
        model_compatibility_reason: None,
    })
}

fn print_config_summary(config: &Config, path: &std::path::Path) {
    let detected_backend = resolved_backend(config);
    let power = probe_power_state();
    let active_model = effective_active_model(config);
    let agent = latest_agent_state(config);
    let identity_ready = identity_ready()
        || agent
            .as_ref()
            .and_then(|state| state.identity_trust_path.as_ref())
            .is_some();
    let fallback_allowed = policy_allowed(config, &power, active_model.as_deref(), identity_ready);
    let allowed = agent
        .as_ref()
        .and_then(|state| state.policy_allowed)
        .unwrap_or(fallback_allowed);
    let readiness = agent
        .as_ref()
        .and_then(live_readiness_from_agent)
        .unwrap_or_else(|| {
            local_readiness(config, &power, active_model.as_deref(), identity_ready)
        });
    let provider_count = if readiness.ready_for_jobs { 1 } else { 0 };
    theme::section("Node status");
    theme::field("configPath", path.display());
    theme::field("deviceId", &config.device_id);
    theme::field("publicKey", display_public_key_hex(config));
    theme::field(
        "publicKeyFingerprint",
        display_public_key_fingerprint(config),
    );
    theme::field(
        "profileName",
        config.profile_name.as_deref().unwrap_or("unset"),
    );
    theme::field(
        "authenticated",
        if auth_token::operator_token_present(config) {
            "yes"
        } else {
            "no"
        },
    );
    theme::field(
        "connected",
        colored_state(config.connected, Color::Green, "yes", "no"),
    );
    theme::field(
        "paused",
        colored_state(config.paused, Color::AnsiValue(208), "yes", "no"),
    );
    theme::field("backendPreference", config.backend_preference);
    theme::field("detectedBackend", detected_backend);
    theme::field(
        "runtimePreference",
        config
            .runtime_preference
            .as_deref()
            .unwrap_or("platform-default"),
    );
    theme::field(
        "fallbackRuntime",
        config.fallback_runtime.as_deref().unwrap_or("none"),
    );
    theme::field("identityReady", theme::boolean(identity_ready, "yes", "no"));
    theme::field("identityTrustPath", identity::trust_path());
    theme::field("providerCount", provider_count);
    theme::field("modelDir", configured_model_dir_string(config));
    match config.contributed_cluster.as_ref() {
        Some(cluster) => theme::field(
            "contributedCluster",
            format!("{} at {}", cluster.kind, cluster.base_url),
        ),
        None => theme::field("contributedCluster", "none"),
    }
    theme::field(
        "activeModel",
        active_model.clone().unwrap_or_else(|| "unset".to_string()),
    );
    theme::field(
        "contributionPercent",
        if config.contribution_percent == 0 {
            "unset".to_string()
        } else {
            format!("{}%", config.contribution_percent)
        },
    );
    theme::field("controlPlaneUrl", &config.control_plane_url);
    theme::field("powerSource", &power.source);
    theme::field("onBattery", theme::boolean(power.on_battery, "yes", "no"));
    theme::field(
        "batteryPercent",
        power
            .battery_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "unknown".to_string()),
    );
    theme::field("policyAllowed", theme::boolean(allowed, "yes", "no"));
    if let Some(reason) = policy_reason(config, &power, active_model.as_deref(), identity_ready) {
        theme::field("policyReason", reason);
    }
    theme::field(
        "readyForJobs",
        theme::boolean(readiness.ready_for_jobs, "yes", "no"),
    );
    if let Some(reason) = readiness.readiness_reason.as_ref() {
        theme::field("readinessReason", reason);
    }
    if let Some(compatibility) = readiness.model_compatibility.as_ref() {
        theme::field("activeModelCompatibility", compatibility);
    }
    if let Some(reason) = readiness.model_compatibility_reason.as_ref() {
        theme::field("activeModelCompatibilityReason", reason);
    }
    theme::field(
        "onboardingCompleted",
        if config.onboarding_completed {
            "yes"
        } else {
            "no"
        },
    );
}

fn print_startup_summary(config: &Config, path: &std::path::Path) {
    let detected_backend = resolved_backend(config);
    let cores = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let power = probe_power_state();
    let active_model = effective_active_model(config);
    let identity_ready = identity_ready();
    let allowed = policy_allowed(config, &power, active_model.as_deref(), identity_ready);
    let readiness = local_readiness(config, &power, active_model.as_deref(), identity_ready);

    theme::section("Startup ready");
    theme::field("deviceId", &config.device_id);
    theme::field("publicKey", display_public_key_hex(config));
    theme::field(
        "publicKeyFingerprint",
        display_public_key_fingerprint(config),
    );
    theme::field(
        "platform",
        format!("{}-{}", env::consts::OS, env::consts::ARCH),
    );
    theme::field("cpuCores", cores);
    theme::field("backendPreference", config.backend_preference);
    theme::field("detectedBackend", detected_backend);
    theme::field(
        "runtimePreference",
        config
            .runtime_preference
            .as_deref()
            .unwrap_or("platform-default"),
    );
    theme::field(
        "fallbackRuntime",
        config.fallback_runtime.as_deref().unwrap_or("none"),
    );
    theme::field("identityReady", theme::boolean(identity_ready, "yes", "no"));
    theme::field("identityTrustPath", identity::trust_path());
    theme::field("modelDir", configured_model_dir_string(config));
    match config.contributed_cluster.as_ref() {
        Some(cluster) => theme::field(
            "contributedCluster",
            format!("{} at {}", cluster.kind, cluster.base_url),
        ),
        None => theme::field("contributedCluster", "none"),
    }
    theme::field(
        "activeModel",
        active_model.clone().unwrap_or_else(|| "unset".to_string()),
    );
    theme::field(
        "contributionPercent",
        if config.contribution_percent == 0 {
            "unset".to_string()
        } else {
            format!("{}%", config.contribution_percent)
        },
    );
    theme::field(
        "connected",
        colored_state(config.connected, Color::Green, "yes", "no"),
    );
    theme::field(
        "paused",
        colored_state(config.paused, Color::AnsiValue(208), "yes", "no"),
    );
    theme::field("configPath", path.display());
    theme::field("powerSource", &power.source);
    theme::field("onBattery", theme::boolean(power.on_battery, "yes", "no"));
    theme::field(
        "batteryPercent",
        power
            .battery_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "unknown".to_string()),
    );
    theme::field("policyAllowed", theme::boolean(allowed, "yes", "no"));
    if let Some(reason) = policy_reason(config, &power, active_model.as_deref(), identity_ready) {
        theme::field("policyReason", reason);
    }
    theme::field(
        "readyForJobs",
        theme::boolean(readiness.ready_for_jobs, "yes", "no"),
    );
    if let Some(reason) = readiness.readiness_reason.as_ref() {
        theme::field("readinessReason", reason);
    }
    if let Some(compatibility) = readiness.model_compatibility.as_ref() {
        theme::field("activeModelCompatibility", compatibility);
    }
    if let Some(reason) = readiness.model_compatibility_reason.as_ref() {
        theme::field("activeModelCompatibilityReason", reason);
    }
    theme::field(
        "onboardingCompleted",
        if config.onboarding_completed {
            "yes"
        } else {
            "no"
        },
    );
}

fn companion_node_agent_name() -> &'static str {
    if cfg!(windows) {
        "opengpu-node-agent.exe"
    } else {
        "opengpu-node-agent"
    }
}

fn resolve_node_agent_executable() -> PathBuf {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let companion = parent.join(companion_node_agent_name());
            if companion.exists() {
                return companion;
            }
        }
    }

    PathBuf::from(companion_node_agent_name())
}

fn node_agent_pid_path() -> PathBuf {
    config::config_dir().join("node-agent.pid")
}

fn node_agent_log_path() -> PathBuf {
    config::config_dir().join("node-agent.log")
}

fn node_agent_error_log_path() -> PathBuf {
    config::config_dir().join("node-agent.err.log")
}

fn write_node_agent_pid(pid: u32) -> Result<(), String> {
    let path = node_agent_pid_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create agent pid directory: {error}"))?;
    }
    std::fs::write(&path, pid.to_string()).map_err(|error| {
        format!(
            "failed to write agent pid file `{}`: {error}",
            path.display()
        )
    })
}

fn remove_node_agent_pid() {
    let _ = std::fs::remove_file(node_agent_pid_path());
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .stdin(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Returns Ok(true) if a running process was found and stopped, Ok(false) if
/// the pid was already gone (nothing to do), Err if the stop attempt failed.
fn stop_process_by_pid(pid: u32) -> Result<bool, String> {
    if !is_process_alive(pid) {
        return Ok(false);
    }

    #[cfg(windows)]
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("failed to run taskkill for agent pid {pid}: {error}"))?;

    #[cfg(not(windows))]
    let status = Command::new("kill")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("failed to run kill for agent pid {pid}: {error}"))?;

    if status.success() {
        Ok(true)
    } else {
        Err(format!("agent stop command exited with {status}"))
    }
}

fn read_node_agent_pid() -> Result<Option<u32>, String> {
    let path = node_agent_pid_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "failed to read agent pid file `{}`: {error}",
            path.display()
        )
    })?;
    let pid = raw
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid agent pid file `{}`: {error}", path.display()))?;
    Ok(Some(pid))
}

fn stop_background_node_agent() -> Result<Option<u32>, String> {
    let Some(pid) = read_node_agent_pid()? else {
        return Ok(None);
    };

    let result = stop_process_by_pid(pid);
    remove_node_agent_pid();
    match result {
        Ok(true) => Ok(Some(pid)),
        Ok(false) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Warns if the last recorded session ended without going through the
/// normal shutdown path (crash, forced kill, closed terminal, etc.), using
/// whatever state it last reported before going silent.
fn report_stale_previous_session() {
    let Ok(Some(pid)) = read_node_agent_pid() else {
        return;
    };
    if is_process_alive(pid) {
        return;
    }

    let state_path = config::config_dir().join("agent-state.json");
    let Ok(raw) = std::fs::read_to_string(&state_path) else {
        return;
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    let agent_state = state
        .get("agent_state")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let age_desc = state
        .get("updated_at")
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|updated_at| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs() as i64;
            Some(format!(" {}s ago", (now - updated_at).max(0)))
        })
        .unwrap_or_default();

    theme::warn(format!(
        "Previous node session (pid {pid}) ended without a clean shutdown; last state was `{agent_state}`{age_desc}"
    ));
}

fn send_node_agent_stop() -> Result<(), String> {
    let agent = resolve_node_agent_executable();
    let status = Command::new(&agent)
        .arg("stop")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            format!(
                "failed to run node agent stop `{}`: {error}",
                agent.display()
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("node agent stop exited with {status}"))
    }
}

fn disconnect_node_agent() -> Result<Option<u32>, String> {
    let stop_result = send_node_agent_stop();
    let process_result = stop_background_node_agent();

    match (stop_result, process_result) {
        (Ok(()), Ok(pid)) => Ok(pid),
        (Err(stop_error), Ok(pid)) => {
            if pid.is_some() {
                Err(format!(
                    "sent local stop but failed to publish final heartbeat: {stop_error}"
                ))
            } else {
                Err(stop_error)
            }
        }
        (Ok(()), Err(process_error)) => Err(process_error),
        (Err(stop_error), Err(process_error)) => Err(format!(
            "{stop_error}; also failed to stop process: {process_error}"
        )),
    }
}

#[derive(Clone, Copy)]
enum AgentLaunchMode {
    Foreground,
    ForegroundDebug,
    Background,
}

struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn enable_session_raw_mode() -> Option<RawModeGuard> {
    match enable_raw_mode() {
        Ok(()) => Some(RawModeGuard),
        Err(error) => {
            theme::warn(format!("Could not enable interactive terminal input: {error}"));
            None
        }
    }
}

fn mark_disconnected() -> Result<Config, String> {
    let mut config = current_config_or_default();
    config.connected = false;
    config.paused = true;
    save_config(&config).map_err(|error| format!("failed to save disconnect state: {error}"))?;
    Ok(config)
}

fn tee_stream<R: Read, W: Write>(mut reader: R, mut mirror: W, mut log: std::fs::File) {
    let mut buffer = [0u8; 4096];
    let mut previous_was_carriage_return = false;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let terminal_bytes =
                    terminal_line_endings(&buffer[..n], &mut previous_was_carriage_return);
                let _ = mirror.write_all(&terminal_bytes);
                let _ = mirror.flush();
                let _ = log.write_all(&buffer[..n]);
                let _ = log.flush();
            }
        }
    }
}

fn terminal_line_endings(bytes: &[u8], previous_was_carriage_return: &mut bool) -> Vec<u8> {
    let mut rendered = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte == b'\n' && !*previous_was_carriage_return {
            rendered.push(b'\r');
        }
        rendered.push(byte);
        *previous_was_carriage_return = byte == b'\r';
    }
    rendered
}

fn relay_background_startup_output(log_path: &Path, offset: &mut u64) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(log_path)
        .map_err(|error| format!("failed to read agent log `{}`: {error}", log_path.display()))?;
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(*offset))
        .map_err(|error| format!("failed to seek agent log `{}`: {error}", log_path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("failed to tail agent log `{}`: {error}", log_path.display()))?;
    *offset = offset.saturating_add(bytes.len() as u64);

    for line in String::from_utf8_lossy(&bytes).split(['\r', '\n']) {
        let line = line.trim();
        if line.starts_with("vllmStartup:") {
            println!("{line}");
        }
    }
    Ok(())
}

fn background_worker_is_healthy(previous_state: Option<&str>) -> bool {
    let state_path = config::config_dir().join("agent-state.json");
    let Ok(raw) = std::fs::read_to_string(state_path) else {
        return false;
    };
    if previous_state == Some(raw.as_str()) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|state| state["worker_health"]["healthy"].as_bool())
        .unwrap_or(false)
}

fn wait_for_background_agent_startup(
    child: &mut std::process::Child,
    agent: &Path,
    log_path: &Path,
    error_log_path: &Path,
    mut log_offset: u64,
    previous_state: Option<&str>,
) -> Result<(), String> {
    let timeout_seconds = env::var("OPENGPU_AGENT_START_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1800);
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    while Instant::now() < deadline {
        relay_background_startup_output(log_path, &mut log_offset)?;
        if background_worker_is_healthy(previous_state) {
            theme::field("agent", theme::status("ready"));
            return Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|error| {
            format!(
                "failed to inspect node agent `{}`: {error}",
                agent.display()
            )
        })? {
            remove_node_agent_pid();
            return Err(format!(
                "node agent exited during startup with {status}; see `{}` and `{}`",
                log_path.display(),
                error_log_path.display()
            ));
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "node agent did not become healthy within {timeout_seconds}s; it remains in the background; see `{}`",
        log_path.display()
    ))
}

fn run_node_agent_foreground(
    mut command: Command,
    agent: PathBuf,
    debug: bool,
) -> Result<(), String> {
    if debug {
        command.arg("--verbose");
    }

    let log_path = node_agent_log_path();
    let error_log_path = node_agent_error_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create agent log directory: {error}"))?;
    }
    let stdout_log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("failed to open agent log `{}`: {error}", log_path.display()))?;
    let stderr_log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&error_log_path)
        .map_err(|error| {
            format!(
                "failed to open agent error log `{}`: {error}",
                error_log_path.display()
            )
        })?;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }

    theme::section("Starting OpenGPU");
    theme::field(
        "mode",
        if debug { "foreground debug" } else { "foreground" },
    );
    theme::field("command", format!("{} run", agent.display()));
    theme::field("log", log_path.display());
    theme::field("error log", error_log_path.display());
    theme::note("Press Esc or Ctrl-C to disconnect");

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run node agent `{}`: {error}", agent.display()))?;
    let pid = child.id();
    write_node_agent_pid(pid)?;

    let stdout_pipe = child.stdout.take().expect("node agent stdout piped");
    let stderr_pipe = child.stderr.take().expect("node agent stderr piped");
    let stdout_tee = thread::spawn(move || tee_stream(stdout_pipe, io::stdout(), stdout_log));
    let stderr_tee = thread::spawn(move || tee_stream(stderr_pipe, io::stderr(), stderr_log));

    let mut raw_mode = enable_session_raw_mode();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            format!(
                "failed to inspect node agent `{}`: {error}",
                agent.display()
            )
        })? {
            remove_node_agent_pid();
            let _ = stdout_tee.join();
            let _ = stderr_tee.join();
            if status.success() {
                return Ok(());
            }
            return Err(format!("node agent exited with {status}"));
        }

        match poll(Duration::from_millis(200)) {
            Ok(true) => match read() {
                Ok(Event::Key(event))
                    if handles_terminal_key(event.kind)
                        && (event.code == KeyCode::Esc
                            || event.code == KeyCode::Char('c')
                                && event.modifiers.contains(KeyModifiers::CONTROL)) =>
                {
                    mark_disconnected()?;
                    drop(raw_mode.take());
                    if let Err(error) = send_node_agent_stop() {
                        theme::warn(format!("Could not stop the node agent cleanly: {error}"));
                    }
                    let _ = stop_process_by_pid(pid);
                    remove_node_agent_pid();
                    let _ = stdout_tee.join();
                    let _ = stderr_tee.join();
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) => {
                    theme::warn(format!("Could not read terminal input: {error}"))
                }
            },
            Ok(false) => {}
            Err(error) => theme::warn(format!("Could not poll terminal input: {error}")),
        }
    }
}

fn launch_node_agent(mode: AgentLaunchMode) -> Result<(), String> {
    let agent = resolve_node_agent_executable();
    let mut command = Command::new(&agent);
    command.arg("run");

    report_stale_previous_session();

    if let Err(error) = stop_background_node_agent() {
        theme::warn(format!("Could not stop the previous node agent: {error}"));
    }

    match mode {
        AgentLaunchMode::Foreground => {
            return run_node_agent_foreground(command, agent, false);
        }
        AgentLaunchMode::ForegroundDebug => {
            return run_node_agent_foreground(command, agent, true);
        }
        AgentLaunchMode::Background => {}
    }

    let log_path = node_agent_log_path();
    let error_log_path = node_agent_error_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create agent log directory: {error}"))?;
    }
    let log_offset = std::fs::metadata(&log_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let previous_state =
        std::fs::read_to_string(config::config_dir().join("agent-state.json")).ok();
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("failed to open agent log `{}`: {error}", log_path.display()))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&error_log_path)
        .map_err(|error| {
            format!(
                "failed to open agent error log `{}`: {error}",
                error_log_path.display()
            )
        })?;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start node agent `{}`: {error}", agent.display()))?;
    let pid = child.id();

    thread::sleep(Duration::from_millis(750));
    if let Some(status) = child.try_wait().map_err(|error| {
        format!(
            "failed to inspect node agent `{}`: {error}",
            agent.display()
        )
    })? {
        remove_node_agent_pid();
        return Err(format!(
            "node agent exited immediately with {status}; see `{}` and `{}`",
            log_path.display(),
            error_log_path.display()
        ));
    }

    if let Err(error) = write_node_agent_pid(pid) {
        let _ = stop_process_by_pid(pid);
        return Err(error);
    }
    theme::section("Starting OpenGPU");
    theme::field("agent", theme::status("running"));
    theme::field("pid", pid);
    theme::field("mode", "background");
    theme::field("log", log_path.display());
    theme::field("error log", error_log_path.display());
    wait_for_background_agent_startup(
        &mut child,
        &agent,
        &log_path,
        &error_log_path,
        log_offset,
        previous_state.as_deref(),
    )
}

fn current_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn print_onboarding_checklist(config: &Config, path: &std::path::Path, completed: bool) {
    let detected_backend = resolved_backend(config);
    let power = probe_power_state();
    let active_model = effective_active_model(config);
    let identity_ready = identity_ready();
    let allowed = policy_allowed(config, &power, active_model.as_deref(), identity_ready);
    let body = vec![
        format!("device id: {}", config.device_id),
        format!(
            "identity: {}",
            if identity_ready {
                "ready"
            } else {
                "unavailable"
            }
        ),
        format!("device key: {}", display_public_key_fingerprint(config)),
        format!("hostname: {}", current_hostname()),
        format!("backend: {}", detected_backend),
        format!(
            "active model: {}",
            active_model.clone().unwrap_or_else(|| "unset".to_string())
        ),
        format!(
            "contribution cap: {}",
            if config.contribution_percent == 0 {
                "unset".to_string()
            } else {
                format!("{}%", config.contribution_percent)
            }
        ),
        format!("policy: {}", if allowed { "allowed" } else { "blocked" }),
        format!("credits: /v1/credits"),
        format!("dashboard: http://127.0.0.1:3001"),
        format!("config: {}", path.display()),
        format!(
            "state: {}",
            if completed {
                "complete"
            } else {
                "review needed"
            }
        ),
    ];
    print_retro_panel(
        "CONTRIBUTOR ONBOARDING",
        "review the trust, model, policy, and credits setup",
        &body,
        if completed { Color::Green } else { Color::Cyan },
    );
}

fn onboarding_completion_hint(
    config: &Config,
    identity_ready: bool,
    active_model: Option<&str>,
    policy_allowed: bool,
) -> &'static str {
    if !identity_ready {
        return "run `opengpu start` to create and verify the secure device identity";
    }

    if active_model.is_none() {
        return "run `opengpu start` to cache the starter model";
    }

    if config.contribution_percent == 0 {
        return "run `opengpu cap` to choose the contribution budget";
    }

    if !config.connected || config.paused {
        return "run `opengpu start` to bring the node online";
    }

    if !policy_allowed {
        return "the machine is configured, but policy is currently blocking work";
    }

    "the machine is ready for contributor use"
}

fn print_model_inventory(config: &Config, models: &[ModelRecord], json: bool) {
    if json {
        let payload = serde_json::json!({
            "model_dir": configured_model_dir_string(config),
            "active_model": active_model_name(config),
            "models": models,
        });
        if let Err(error) = print_json(&payload) {
            eprintln!("failed to print json: {error}");
            std::process::exit(1);
        }
        return;
    }

    let active_model = active_model_name(config).unwrap_or_else(|| "unset".to_string());
    let subtitle = format!("active model: {}", active_model);
    let mut body = vec![
        format!("model dir: {}", configured_model_dir_string(config)),
        format!("cache size: {} model(s)", models.len()),
    ];

    if models.is_empty() {
        body.push("models: none cached".to_string());
    } else {
        body.push("cached models:".to_string());
        for model in models {
            let state = if model.active { "ACTIVE" } else { "cached" };
            let prefix = if model.active { ">>" } else { "  " };
            let compatibility = model.compatibility.as_deref().unwrap_or("unknown");
            body.push(format!(
                "{prefix} {:<28} [{state}, {compatibility}]",
                model.name
            ));
            if let Some(reason) = model.compatibility_reason.as_ref() {
                body.push(format!("     reason: {reason}"));
            }
        }
    }

    print_retro_panel("MODEL CACHE", &subtitle, &body, Color::Cyan);
}

fn print_model_event(title: &str, model_name: &str, detail: &str, accent: Color, config: &Config) {
    let body = vec![
        format!("model: {}", model_name),
        format!("detail: {}", detail),
        format!("model dir: {}", configured_model_dir_string(config)),
        format!(
            "active model: {}",
            active_model_name(config).unwrap_or_else(|| "unset".to_string())
        ),
    ];
    print_retro_panel(title, "local cache updated", &body, accent);
}

fn read_operator_token_from_prompt() -> Result<String, String> {
    if !io::stdin().is_terminal() {
        return Err("missing token; pass --token or use an interactive terminal".to_string());
    }

    print!("Operator token: ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut token = String::new();
    io::stdin()
        .read_line(&mut token)
        .map_err(|error| error.to_string())?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("operator token cannot be empty".to_string());
    }
    Ok(token)
}

fn print_retro_panel(title: &str, subtitle: &str, lines: &[String], accent: Color) {
    theme::panel(title, subtitle, lines, accent);
}

fn contribution_semantics(backend: Backend) -> &'static str {
    match backend {
        Backend::M => "memory-and-compute budget for Apple Silicon M-series",
        Backend::Cuda => "automatic routing budget",
        Backend::Vulkan => "shared-memory Vulkan acceleration budget",
        Backend::Vllm => "Linux vLLM routing budget",
        Backend::Auto => "automatic routing budget",
    }
}

fn default_contribution_percent(backend: Backend) -> u8 {
    match backend {
        Backend::Cuda => 30,
        Backend::Vulkan => 30,
        Backend::Vllm => 30,
        Backend::M => 30,
        Backend::Auto => 20,
    }
}

fn detect_backend() -> Backend {
    if env::consts::OS == "macos" && env::consts::ARCH == "aarch64" {
        return Backend::M;
    }

    if env::var_os("NVIDIA_VISIBLE_DEVICES").is_some()
        || env::var_os("CUDA_VISIBLE_DEVICES").is_some()
        || Command::new("nvidia-smi")
            .arg("--query-gpu=name")
            .arg("--format=csv,noheader")
            .output()
            .map(|output| output.status.success() && !output.stdout.is_empty())
            .unwrap_or(false)
    {
        return Backend::Cuda;
    }

    if env::consts::OS == "windows" {
        let llama_cli = env::var_os("OPENGPU_LLAMA_CLI")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                config::config_dir()
                    .join("runtimes")
                    .join("llama")
                    .join("llama-cli.exe")
            });
        if Command::new(llama_cli)
            .arg("--list-devices")
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout)
                        .to_ascii_lowercase()
                        .contains("vulkan")
            })
            .unwrap_or(false)
        {
            return Backend::Vulkan;
        }
    }

    Backend::Auto
}

fn detect_cuda_gpu_name() -> Option<String> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

fn detect_cuda_vram_mb() -> Option<u64> {
    let output = Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()
}

fn install_profile_for(os: &str, arch: &str, backend: Backend) -> &'static str {
    match (os, arch, backend) {
        ("macos", "aarch64", Backend::M) => "macos-aarch64-apple-silicon",
        ("windows", "x86_64", Backend::Cuda) => "windows-x86_64-cuda",
        ("windows", "x86_64", Backend::Vulkan) => "windows-x86_64-vulkan",
        ("linux", "x86_64", Backend::Cuda) => "linux-x86_64-cuda",
        ("linux", "aarch64", Backend::Cuda) => "linux-aarch64-cuda",
        ("linux", "x86_64", Backend::Vllm) => "linux-x86_64-vllm",
        ("linux", "aarch64", Backend::Vllm) => "linux-aarch64-vllm",
        ("windows", "x86_64", _) => "windows-x86_64-generic",
        ("linux", "x86_64", _) => "linux-x86_64-generic",
        ("linux", "aarch64", _) => "linux-aarch64-generic",
        ("macos", "x86_64", _) => "macos-x86_64-generic",
        _ => "unsupported-or-generic",
    }
}

fn opengpu_home_dir() -> PathBuf {
    std::env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn is_apple_silicon_macos() -> bool {
    env::consts::OS == "macos" && env::consts::ARCH == "aarch64"
}

fn mlx_runtime_python_path() -> PathBuf {
    let mut path = opengpu_home_dir().join("runtimes").join("mlx").join("venv");
    #[cfg(windows)]
    {
        path = path.join("Scripts").join("python.exe");
    }
    #[cfg(not(windows))]
    {
        path = path.join("bin").join("python");
    }
    path
}

#[cfg(target_os = "macos")]
fn install_macos_python3_if_missing() -> Result<(), String> {
    if command_available("python3", &["--version"]) {
        return Ok(());
    }

    if std::env::var("OPENGPU_SKIP_PYTHON_BOOTSTRAP")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
    {
        return Err(
            "python3 is unavailable and OPENGPU_SKIP_PYTHON_BOOTSTRAP is enabled".to_string(),
        );
    }

    let installer_url = std::env::var("OPENGPU_PYTHON_INSTALLER_URL").unwrap_or_else(|_| {
        "https://www.python.org/ftp/python/3.12.4/python-3.12.4-macos11.pkg".to_string()
    });
    let package_path = std::env::temp_dir().join("opengpu-python3-macos.pkg");

    println!("pythonBootstrap: python3 unavailable; downloading official Python package");
    println!("pythonBootstrapUrl: {installer_url}");

    let mut curl = Command::new("/usr/bin/curl");
    curl.args(["-fL", "--retry", "3", "-o"])
        .arg(&package_path)
        .arg(&installer_url);
    run_checked_command(curl, "download Python for MLX")?;

    println!("pythonBootstrap: installing Python package");
    let mut installer = if command_available("id", &["-u"]) {
        let output = Command::new("id").arg("-u").output().ok();
        let is_root = output
            .as_ref()
            .and_then(|output| String::from_utf8(output.stdout.clone()).ok())
            .map(|uid| uid.trim() == "0")
            .unwrap_or(false);
        if is_root {
            Command::new("/usr/sbin/installer")
        } else {
            let mut command = Command::new("/usr/bin/sudo");
            command.arg("/usr/sbin/installer");
            command
        }
    } else {
        let mut command = Command::new("/usr/bin/sudo");
        command.arg("/usr/sbin/installer");
        command
    };
    installer
        .args(["-pkg"])
        .arg(&package_path)
        .args(["-target", "/"]);
    run_checked_command(installer, "install Python for MLX")?;

    if command_available("python3", &["--version"]) {
        Ok(())
    } else {
        Err("Python package installed, but python3 is still unavailable on PATH".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn install_macos_python3_if_missing() -> Result<(), String> {
    Ok(())
}

fn run_checked_command(mut command: Command, action: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("{action} failed to launch: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .or_else(|| stdout.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or("no output");
    Err(format!("{action} failed: {detail}"))
}

fn run_streaming_command(mut command: Command, action: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("{action} failed to launch: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{action} failed with exit status {}",
            status.code().unwrap_or(-1)
        ))
    }
}

fn is_hugging_face_model_id(model: &str) -> bool {
    let mut parts = model.trim().split('/');
    let Some(owner) = parts.next() else {
        return false;
    };
    let Some(name) = parts.next() else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && parts.next().is_none()
        && !owner.starts_with('.')
        && !name.starts_with('.')
        && owner
            .chars()
            .chain(name.chars())
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn prefetch_mlx_model(python: &Path, model: &str) -> Result<(), String> {
    if !is_hugging_face_model_id(model) {
        return Err(format!(
            "active model `{model}` is not a Hugging Face repository ID"
        ));
    }

    println!("modelPrefetch: caching {model} for MLX");
    let script = concat!(
        "import sys; ",
        "from huggingface_hub import snapshot_download; ",
        "path=snapshot_download(repo_id=sys.argv[1]); ",
        "snapshot_download(repo_id=sys.argv[1], local_files_only=True); ",
        "print(f'modelPrefetchPath: {path}')"
    );
    let mut command = Command::new(python);
    command.args(["-c", script, model]);
    run_streaming_command(command, "prefetch MLX model")?;
    println!("modelPrefetch: ready");
    Ok(())
}

fn prefetch_active_mlx_model(config: &Config, python: &Path) -> Result<(), String> {
    let model = active_model_name(config)
        .ok_or_else(|| "an active model must be selected before installing MLX".to_string())?;
    prefetch_mlx_model(python, &model)
}

fn prefetch_mlx_catalog_model(config: &Config, model: &str) -> Result<(), String> {
    if config.runtime_preference.as_deref() != Some("mlx") {
        return Ok(());
    }
    let Some(option) = lookup_model_for_backend(model, Backend::M) else {
        return Ok(());
    };
    if option.source_kind != "huggingface-mlx" {
        return Ok(());
    }
    let python = verify_mlx_runtime()?;
    prefetch_mlx_model(&python, model)
}

fn vllm_runtime_config_value(key: &str) -> Option<String> {
    let path = config_dir()
        .join("runtimes")
        .join("vllm")
        .join("runtime.conf");
    let contents = std::fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate.trim() == key)
            .then(|| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn should_prefetch_vllm_catalog_model(config: &Config, model: &str) -> bool {
    resolved_backend(config) == Backend::Vllm
        && lookup_model_for_backend(model, Backend::Vllm)
            .map(|option| option.source_kind == "huggingface-vllm")
            .unwrap_or(false)
}

fn prefetch_vllm_catalog_model(config: &Config, model: &str) -> Result<(), String> {
    if !should_prefetch_vllm_catalog_model(config, model) {
        return Ok(());
    }
    if !cfg!(target_os = "linux") {
        return Err("vLLM model downloads are supported only on Linux".to_string());
    }
    if !is_hugging_face_model_id(model) {
        return Err(format!(
            "model `{model}` is not a Hugging Face repository ID"
        ));
    }

    let docker = env::var_os("OPENGPU_DOCKER_BIN").unwrap_or_else(|| "docker".into());
    let image = env::var("OPENGPU_VLLM_IMAGE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| vllm_runtime_config_value("VLLM_IMAGE"))
        .ok_or_else(|| {
            "the vLLM runtime image is not configured; run the Linux installer with --with-vllm"
                .to_string()
        })?;
    let model_dir = PathBuf::from(configured_model_dir_string(config));
    std::fs::create_dir_all(&model_dir)
        .map_err(|error| format!("failed to create model cache directory: {error}"))?;

    println!("modelPrefetch: downloading {model} for vLLM");
    println!("modelPrefetchCache: {}", model_dir.display());
    let script = concat!(
        "import sys; ",
        "from huggingface_hub import snapshot_download; ",
        "path=snapshot_download(repo_id=sys.argv[1]); ",
        "print(f'\\nmodelPrefetchPath: {path}')"
    );
    let mut command = Command::new(docker);
    command.args([
        "run",
        "--rm",
        "--gpus",
        "all",
        "--ipc=host",
        "--ulimit",
        "memlock=-1",
        "--ulimit",
        "stack=67108864",
    ]);
    if io::stderr().is_terminal() {
        command.arg("-t");
    }
    command
        .arg("-v")
        .arg(format!("{}:/models", model_dir.display()))
        .args(["-e", "HF_HOME=/models/.huggingface"]);
    if env::var_os("HF_TOKEN").is_some() {
        command.args(["-e", "HF_TOKEN"]);
    }
    command.arg(image).args(["python3", "-c", script, model]);
    run_streaming_command(command, "prefetch vLLM model")?;
    println!("modelPrefetch: ready");
    Ok(())
}

fn verify_mlx_runtime() -> Result<PathBuf, String> {
    let python = mlx_runtime_python_path();
    if !python.exists() {
        return Err(format!(
            "MLX runtime python was not found at {}",
            python.display()
        ));
    }

    let mut command = Command::new(&python);
    command.args(["-c", "import mlx_lm; print('mlx-lm ok')"]);
    run_checked_command(command, "verify MLX runtime")?;
    Ok(python)
}

fn install_mlx_runtime(config: &mut Config) -> Result<PathBuf, String> {
    if !is_apple_silicon_macos() {
        return Err("MLX is supported only on Apple Silicon macOS nodes".to_string());
    }

    if let Ok(path) = verify_mlx_runtime() {
        config.runtime_preference = Some("mlx".to_string());
        config.fallback_runtime = Some("llama-metal".to_string());
        prefetch_active_mlx_model(config, &path)?;
        return Ok(path);
    }

    let venv_dir = opengpu_home_dir().join("runtimes").join("mlx").join("venv");
    if !venv_dir.exists() {
        install_macos_python3_if_missing()?;
        let mut command = Command::new("python3");
        command.args(["-m", "venv"]).arg(&venv_dir);
        run_checked_command(command, "create MLX runtime venv")?;
    }

    let python = mlx_runtime_python_path();
    let mut pip_upgrade = Command::new(&python);
    pip_upgrade.args(["-m", "pip", "install", "--upgrade", "pip"]);
    run_checked_command(pip_upgrade, "upgrade MLX runtime pip")?;

    let mut pip_install = Command::new(&python);
    pip_install.args(["-m", "pip", "install", "--upgrade", "mlx-lm"]);
    run_checked_command(pip_install, "install MLX runtime")?;

    let python = verify_mlx_runtime()?;
    config.runtime_preference = Some("mlx".to_string());
    config.fallback_runtime = Some("llama-metal".to_string());
    prefetch_active_mlx_model(config, &python)?;
    Ok(python)
}

fn prompt_mlx_fallback_action(error: &str) -> bool {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return false;
    }

    println!("runtimeInstall: mlx failed");
    println!("runtimeError: {error}");
    println!("runtimeQuestion: retry MLX install? [y/N]");
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

fn configure_macos_runtime(config: &mut Config) {
    if !is_apple_silicon_macos() {
        return;
    }

    println!("runtimePreference: mlx");
    println!("runtimeFallback: llama-metal");
    match install_mlx_runtime(config) {
        Ok(path) => println!("runtimeInstall: mlx ready at {}", path.display()),
        Err(first_error) if prompt_mlx_fallback_action(&first_error) => {
            match install_mlx_runtime(config) {
                Ok(path) => println!("runtimeInstall: mlx ready at {}", path.display()),
                Err(second_error) => {
                    config.runtime_preference = Some("llama-metal".to_string());
                    config.fallback_runtime = Some("mlx".to_string());
                    println!("runtimeInstall: mlx unavailable after retry");
                    println!("runtimeFallback: llama-metal");
                    println!("runtimeError: {second_error}");
                }
            }
        }
        Err(error) => {
            config.runtime_preference = Some("llama-metal".to_string());
            config.fallback_runtime = Some("mlx".to_string());
            println!("runtimeInstall: mlx unavailable");
            println!("runtimeFallback: llama-metal");
            println!("runtimeError: {error}");
            println!("runtimeHint: run `opengpu runtime install mlx` to retry");
        }
    }
}

struct MachineProfile {
    os: &'static str,
    arch: &'static str,
    backend: Backend,
    install_profile: &'static str,
    cuda_gpu_name: Option<String>,
    cuda_vram_mb: Option<u64>,
}

fn detect_machine_profile() -> MachineProfile {
    let detected_backend = detect_backend();
    let vllm_runtime_available = config_dir()
        .join("runtimes")
        .join("vllm")
        .join("runtime.conf")
        .is_file();
    let backend =
        preferred_installed_backend(env::consts::OS, detected_backend, vllm_runtime_available);
    MachineProfile {
        os: env::consts::OS,
        arch: env::consts::ARCH,
        backend,
        install_profile: install_profile_for(env::consts::OS, env::consts::ARCH, backend),
        cuda_gpu_name: if backend == Backend::Cuda {
            detect_cuda_gpu_name()
        } else {
            None
        },
        cuda_vram_mb: if backend == Backend::Cuda {
            detect_cuda_vram_mb()
        } else {
            None
        },
    }
}

fn preferred_installed_backend(
    os: &str,
    detected_backend: Backend,
    vllm_runtime_available: bool,
) -> Backend {
    if os == "linux" && detected_backend == Backend::Cuda && vllm_runtime_available {
        Backend::Vllm
    } else {
        detected_backend
    }
}

fn contribution_vram_budget_mb(
    total_vram_mb: Option<u64>,
    contribution_percent: u8,
) -> Option<u64> {
    total_vram_mb.map(|total| total.saturating_mul(contribution_percent as u64) / 100)
}

fn model_vram_budget_mb(config: &Config, backend: Backend) -> Option<u64> {
    match backend {
        Backend::M | Backend::Vulkan | Backend::Vllm => contribution_vram_budget_mb(
            Some(detect_memory_gb().saturating_mul(1024)),
            config.contribution_percent,
        ),
        Backend::Cuda => {
            contribution_vram_budget_mb(detect_cuda_vram_mb(), config.contribution_percent)
        }
        _ => None,
    }
}

fn ensure_catalog_model_fits_machine(
    name: &str,
    backend: Backend,
    config: &Config,
) -> Result<(), String> {
    let available_vram_mb = model_vram_budget_mb(config, backend);

    ensure_catalog_model_fits(name, backend, available_vram_mb).map_err(|error| error.to_string())
}

enum PromptOutcome {
    Selected(u8),
    /// The "Clusters detected" row was chosen: show the cluster list instead.
    UseCluster,
    Cancelled,
}

enum ControlPlaneChoice {
    Public,
    Private,
}

/// Arrow-key selector shared by the setup menus.
///
/// Returns the chosen index, or `None` when the contributor pressed Esc.
fn select_menu_option(
    header: &[String],
    options: &[(String, String)],
    footer: &str,
    initial: usize,
) -> Option<usize> {
    if options.is_empty() {
        return None;
    }

    let mut selected = initial.min(options.len() - 1);
    if enable_raw_mode().is_err() {
        return Some(selected);
    }
    drain_pending_terminal_events();

    let render_menu = |selected: usize| {
        clear_menu_screen();
        for (index, line) in header.iter().enumerate() {
            if index == 0 {
                raw_println!("{}", theme::menu_title(line));
            } else {
                raw_println!("{}", theme::muted(line));
            }
        }
        for (index, (label, detail)) in options.iter().enumerate() {
            let is_selected = index == selected;
            let marker = theme::menu_marker(is_selected);
            let label = theme::menu_label(label, is_selected);
            if detail.is_empty() {
                raw_println!("{marker} {label}");
            } else {
                raw_println!("{marker} {label}  {}", theme::muted(detail));
            }
        }
        raw_println!();
        raw_println!("{}", theme::hint(footer));
        let _ = io::stdout().flush();
    };

    render_menu(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) if handles_terminal_key(key.kind) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    eprintln!("cancelled");
                    std::process::exit(130);
                }
                KeyCode::Up | KeyCode::BackTab => {
                    selected = selected.saturating_sub(1);
                    render_menu(selected);
                }
                KeyCode::Down | KeyCode::Tab => {
                    if selected + 1 < options.len() {
                        selected += 1;
                    }
                    render_menu(selected);
                }
                KeyCode::Enter => break Some(selected),
                KeyCode::Esc => break None,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };

    let _ = disable_raw_mode();
    result
}

/// Result of the cluster picker shown after the contribution cap.
enum ClusterPickOutcome {
    /// Esc: skip the question for this run without recording an answer.
    Skipped,
    /// `None` was chosen: do not contribute, and stop asking.
    Declined,
    /// Index into the ranked cluster list.
    Picked(usize),
}

/// Picker listing every running cluster, biggest model first, so the
/// contributor chooses which one this node should serve.
fn prompt_cluster_pick(clusters: &[&cluster::DetectedCluster]) -> ClusterPickOutcome {
    let header = vec![
        "A local LLM cluster is already running on this machine.".to_string(),
        "Contribute one to MundusX instead of downloading a model?".to_string(),
        "----------------------------------------------------------".to_string(),
    ];

    let mut options: Vec<(String, String)> = clusters
        .iter()
        .map(|entry| {
            let model = entry
                .largest_model()
                .map(cluster::ModelInfo::label)
                .unwrap_or_else(|| "no models loaded".to_string());
            (
                format!("{} at {}", entry.kind.label(), entry.base_url),
                format!("{model}; {} available", entry.models.len()),
            )
        })
        .collect();
    options.push((
        "None".to_string(),
        "do not contribute a cluster; choose a MundusX model instead".to_string(),
    ));

    let none_index = options.len() - 1;

    match select_menu_option(
        &header,
        &options,
        "Use ↑/↓ or Tab/Shift+Tab and Enter; Esc skips for now",
        0,
    ) {
        None => ClusterPickOutcome::Skipped,
        Some(index) if index == none_index => ClusterPickOutcome::Declined,
        Some(index) => ClusterPickOutcome::Picked(index),
    }
}

fn prompt_control_plane_choice() -> ControlPlaneChoice {
    const OPTIONS: [(&str, &str); 2] = [
        ("Public MundusX", "use the hosted mundusx.ai control plane"),
        ("Private / custom", "enter your own control-plane URL"),
    ];

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return ControlPlaneChoice::Public;
    }

    let header = vec![
        "Which control plane should this node use?".to_string(),
        "-----------------------------------------".to_string(),
    ];
    let options: Vec<(String, String)> = OPTIONS
        .iter()
        .map(|(label, detail)| (label.to_string(), detail.to_string()))
        .collect();

    match select_menu_option(&header, &options, "Use ↑/↓ or Tab/Shift+Tab and Enter", 0) {
        Some(1) => ControlPlaneChoice::Private,
        // Enter on the public row, or Esc, keeps the public default.
        _ => ControlPlaneChoice::Public,
    }
}

fn read_private_control_plane_url() -> String {
    loop {
        print!("Private control-plane URL (http:// or https://): ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            eprintln!("failed to read control-plane URL");
            std::process::exit(1);
        }
        let url = input.trim();
        if url.is_empty() {
            println!("Private / custom requires a URL. Choose Public MundusX in the previous menu to use the default.");
            continue;
        }
        if is_valid_control_plane_url(url) {
            return url.to_string();
        }
        println!("Enter a full URL, for example http://127.0.0.1:8787 or https://uat.mundusx.ai");
    }
}

fn is_valid_control_plane_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn normalize_control_plane_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(PUBLIC_CONTROL_PLANE_URL.to_string());
    }
    if is_valid_control_plane_url(trimmed) {
        return Ok(trimmed.to_string());
    }
    Err("control-plane URL must start with http:// or https://".to_string())
}

/// Resolves the control-plane URL and, when the menu was shown, the cluster the
/// contributor picked from it. The cluster index refers to `clusters`.
fn resolve_install_control_plane_url(
    public: bool,
    private: bool,
    control_plane_url: Option<String>,
) -> String {
    if public && private {
        eprintln!("choose either --public or --private, not both");
        std::process::exit(2);
    }

    if let Some(url) = control_plane_url {
        let url = normalize_control_plane_url(&url).unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(2);
        });
        return url;
    }

    if public {
        return PUBLIC_CONTROL_PLANE_URL.to_string();
    }

    if private {
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            return read_private_control_plane_url();
        }
        eprintln!("--private requires --control-plane-url in non-interactive mode");
        std::process::exit(2);
    }

    match prompt_control_plane_choice() {
        ControlPlaneChoice::Public => PUBLIC_CONTROL_PLANE_URL.to_string(),
        ControlPlaneChoice::Private => read_private_control_plane_url(),
    }
}

/// A node contributing a running cluster already has a model to serve, so the
/// MundusX model selector stays out of the way.
fn should_prompt_model_selection(config: &Config) -> bool {
    config.contributed_cluster.is_none() && active_model_name(config).is_none()
}

/// The contribution level menu. When running clusters were detected it gains a
/// final "Clusters detected" row that opens the cluster list.
fn prompt_contribution_percent(default_percent: u8, cluster_count: usize) -> PromptOutcome {
    const OPTIONS: &[Option<(u8, &str)>] = &[
        Some((20, "light")),
        Some((30, "balanced")),
        Some((50, "strong")),
        Some((65, "high")),
        Some((80, "maximum")),
        None,
    ];
    let cluster_index = OPTIONS.len();
    let row_count = OPTIONS.len() + usize::from(cluster_count > 0);

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let mut input = String::new();
        if io::stdin().read_to_string(&mut input).is_ok() {
            let choice = input.trim();
            const OPTIONS: [u8; 5] = [20, 30, 50, 65, 80];
            if let Ok(value) = choice.parse::<u16>() {
                if (1..=OPTIONS.len() as u16).contains(&value) {
                    return PromptOutcome::Selected(OPTIONS[value as usize - 1]);
                }
                if let Ok(percent) = normalize_contribution_percent(value) {
                    return PromptOutcome::Selected(percent);
                }
            }
        }
        return PromptOutcome::Selected(default_percent);
    }

    let mut selected = OPTIONS
        .iter()
        .position(|option| {
            option
                .map(|(percent, _)| percent == default_percent)
                .unwrap_or(false)
        })
        .unwrap_or(1);

    if enable_raw_mode().is_err() {
        return PromptOutcome::Selected(default_percent);
    }
    drain_pending_terminal_events();

    let render_menu = |selected: usize| {
        clear_menu_screen();
        raw_println!("{}", theme::menu_title("Contribution level"));
        raw_println!("{}", theme::menu_rule());
        for (index, option) in OPTIONS.iter().enumerate() {
            let is_selected = index == selected;
            let marker = theme::menu_marker(is_selected);
            match option {
                Some((percent, label)) => raw_println!(
                    "{marker} {}  {}",
                    theme::menu_label(format!("{percent:>2}%"), is_selected),
                    theme::muted(label)
                ),
                None => raw_println!(
                    "{marker} {}  {}",
                    theme::menu_label("Custom", is_selected),
                    theme::muted("type an exact percent (1–80)")
                ),
            }
        }
        if cluster_count > 0 {
            let is_selected = selected == cluster_index;
            let marker = theme::menu_marker(is_selected);
            raw_println!(
                "{marker} {}  {}",
                theme::menu_label(format!("Clusters detected ({cluster_count})"), is_selected),
                theme::muted("show all and pick one to contribute")
            );
        }
        raw_println!();
        raw_println!(
            "{}",
            theme::hint("Use ↑/↓ or Tab/Shift+Tab and Enter, or press 1–5")
        );
        let _ = io::stdout().flush();
    };

    render_menu(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) if handles_terminal_key(key.kind) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    return PromptOutcome::Cancelled;
                }
                KeyCode::Up | KeyCode::BackTab => {
                    selected = selected.saturating_sub(1);
                    render_menu(selected);
                }
                KeyCode::Down | KeyCode::Tab => {
                    if selected + 1 < row_count {
                        selected += 1;
                    }
                    render_menu(selected);
                }
                KeyCode::Char(value) if value.is_ascii_digit() => {
                    if let Some(index) = value
                        .to_digit(10)
                        .and_then(|value| usize::try_from(value).ok())
                        .and_then(|value| value.checked_sub(1))
                    {
                        if index < OPTIONS.len() {
                            selected = index;
                            render_menu(selected);
                        }
                    }
                }
                KeyCode::Enter if cluster_count > 0 && selected == cluster_index => {
                    let _ = disable_raw_mode();
                    return PromptOutcome::UseCluster;
                }
                KeyCode::Enter => match OPTIONS[selected] {
                    Some((percent, _)) => break Some(percent),
                    None => {
                        let _ = disable_raw_mode();
                        match read_custom_contribution_percent() {
                            Some(value) => break Some(value),
                            None => {
                                if enable_raw_mode().is_err() {
                                    break Some(default_percent);
                                }
                                render_menu(selected);
                            }
                        }
                    }
                },
                KeyCode::Esc => break None,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };

    let _ = disable_raw_mode();
    match result {
        Some(value) => PromptOutcome::Selected(value),
        None => PromptOutcome::Selected(default_percent),
    }
}

fn read_custom_contribution_percent() -> Option<u8> {
    loop {
        print!("Custom contribution percent (1-80, blank to go back): ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            eprintln!("failed to read contribution percent");
            return None;
        }
        let value = input.trim();
        if value.is_empty() {
            return None;
        }
        match value.parse::<u16>() {
            Ok(value) => match normalize_contribution_percent(value) {
                Ok(value) => return Some(value),
                Err(error) => println!("{error}"),
            },
            Err(_) => {
                println!("contribution cap must be a whole number from 1 to 80");
            }
        }
    }
}

fn normalize_contribution_percent(percent: u16) -> Result<u8, String> {
    if (1..=80).contains(&percent) {
        Ok(percent as u8)
    } else {
        Err("contribution cap must be between 1 and 80".to_string())
    }
}

fn cap_label(percent: u8) -> &'static str {
    match percent {
        20 => "light",
        30 => "balanced",
        50 => "strong",
        65 => "high",
        80 => "maximum",
        _ => "custom",
    }
}

fn print_contribution_cap(config: &Config, selected: Option<u8>, completed: bool) {
    let current = selected
        .or_else(|| (config.contribution_percent > 0).then_some(config.contribution_percent))
        .unwrap_or(0);
    let current_text = if current == 0 {
        "unset".to_string()
    } else {
        format!("{}% ({})", current, cap_label(current))
    };
    let body = vec![
        format!("current cap: {}", current_text),
        format!(
            "meaning: {}",
            contribution_semantics(resolved_backend(config))
        ),
        "quick caps: 20 / 30 / 50 / 65 / 80; custom caps: 1-80".to_string(),
        "install page: localhost preview at http://127.0.0.1:3002/install".to_string(),
        "next step: run `opengpu start` after saving a cap".to_string(),
        format!(
            "state: {}",
            if completed {
                "saved"
            } else if current == 0 {
                "review needed"
            } else {
                "updated"
            }
        ),
    ];
    print_retro_panel(
        "CONTRIBUTION CAP",
        "choose the budget this Mac is allowed to use",
        &body,
        if current == 0 {
            Color::DarkYellow
        } else {
            Color::Green
        },
    );
}

fn detect_memory_gb() -> u64 {
    #[cfg(target_os = "windows")]
    {
        // CUDA model selection is constrained by dedicated GPU memory, not
        // system RAM. Round down so the displayed class never overstates what
        // the GPU can provide.
        if let Some(vram_mb) = detect_cuda_vram_mb() {
            return vram_mb / 1024;
        }
    }
    #[cfg(target_os = "macos")]
    {
        // sysctl hw.memsize returns total unified memory in bytes
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output();
        if let Ok(out) = output {
            if let Ok(s) = std::str::from_utf8(&out.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return bytes / (1024 * 1024 * 1024);
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        // /proc/meminfo MemTotal in kB
        if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
            for line in contents.lines() {
                if line.starts_with("MemTotal:") {
                    let kb: u64 = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    return kb / (1024 * 1024);
                }
            }
        }
    }
    8 // safe fallback
}

enum ModelChoice {
    Model(ModelOption),
    LocalGguf(String),
}

fn prompt_model_selection(config: &Config, backend: Backend) -> ModelChoice {
    let gb = detect_memory_gb();
    let selection = selection_for(backend, gb);
    let available_vram_mb = model_vram_budget_mb(config, backend);
    let options = if matches!(backend, Backend::Cuda | Backend::Vllm) {
        selectable_catalog_options_for(backend, available_vram_mb)
    } else {
        selectable_options_for(backend, gb, available_vram_mb)
    };
    let allow_local_gguf = backend != Backend::Vllm;
    let choice_count = options.len() + usize::from(allow_local_gguf);

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return options
            .last()
            .cloned()
            .map(ModelChoice::Model)
            .unwrap_or_else(|| ModelChoice::LocalGguf(String::new()));
    }

    let mut selected: usize = options.len().saturating_sub(1);

    if enable_raw_mode().is_err() {
        return options
            .last()
            .cloned()
            .map(ModelChoice::Model)
            .unwrap_or_else(|| ModelChoice::LocalGguf(String::new()));
    }
    drain_pending_terminal_events();

    let render = |selected: usize| {
        clear_menu_screen();
        raw_println!("{}", theme::menu_title("Which model should this node run?"));
        raw_println!(
            "{}",
            theme::muted(format!(
                "Detected {} with {} GB {}",
                selection.backend,
                selection.memory_gb,
                if backend == Backend::Cuda { "VRAM" } else { "memory" }
            ))
        );
        if let Some(budget) = available_vram_mb {
            raw_println!(
                "{}",
                theme::muted(format!(
                    "Model budget {budget} MB VRAM · {}% contribution cap",
                    config.contribution_percent
                ))
            );
        }
        raw_println!("{}", theme::menu_rule());
        for (i, option) in options.iter().enumerate() {
            let is_selected = i == selected;
            let marker = theme::menu_marker(is_selected);
            let estimated = option
                .estimated_vram_mb
                .map(|value| format!("~{:.1} GB runtime", value as f64 / 1024.0))
                .unwrap_or_else(|| "runtime memory unknown".to_string());
            raw_println!(
                "{marker} {}  {}",
                theme::menu_label(format!("{}. {}", i + 1, option.label), is_selected),
                theme::muted(format!("{} · {} · {}", option.name, option.notes, estimated))
            );
        }
        if allow_local_gguf {
            let is_selected = selected == options.len();
            let marker = theme::menu_marker(is_selected);
            raw_println!(
                "{marker} {}",
                theme::menu_label(
                    format!("{}. Import local GGUF / LM Studio model", options.len() + 1),
                    is_selected
                )
            );
        }
        raw_println!();
        raw_println!(
            "{}",
            theme::hint("Use ↑/↓ or Tab/Shift+Tab and Enter · choose one")
        );
        let _ = io::stdout().flush();
    };

    render(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) if handles_terminal_key(key.kind) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    eprintln!("cancelled");
                    std::process::exit(130);
                }
                KeyCode::Up | KeyCode::BackTab => {
                    selected = selected.saturating_sub(1);
                    render(selected);
                }
                KeyCode::Down | KeyCode::Tab => {
                    if selected + 1 < choice_count {
                        selected += 1;
                    }
                    render(selected);
                }
                KeyCode::Enter => break selected,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break selected,
        }
    };

    let _ = disable_raw_mode();

    if allow_local_gguf && result == options.len() {
        clear_menu_screen();
        print!("Path to local .gguf model file: ");
        let _ = io::stdout().flush();
        let mut path = String::new();
        let _ = io::stdin().read_line(&mut path);
        let path = path.trim().to_string();
        if path.is_empty() {
            // still can't skip — fall back to recommended
            ModelChoice::Model(selection.recommended)
        } else {
            ModelChoice::LocalGguf(path)
        }
    } else {
        ModelChoice::Model(options[result].clone())
    }
}

fn prompt_official_model_selection(config: &Config, active: bool) -> ModelOption {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!(
            "model name is required in non-interactive mode; pass `opengpu model {} <official-model-name>`",
            if active { "use" } else { "add" }
        );
        std::process::exit(2);
    }

    let backend = resolved_backend(config);
    let available_vram_mb = model_vram_budget_mb(config, backend);
    let options = selectable_catalog_options_for(backend, available_vram_mb);

    if options.is_empty() {
        eprintln!("no official models fit this machine and contribution cap");
        if backend == Backend::Cuda {
            eprintln!("capHint: raise the cap with `opengpu cap` or check CUDA runtime health");
        }
        std::process::exit(1);
    }

    let mut selected = 0usize;

    if enable_raw_mode().is_err() {
        return options[0].clone();
    }
    drain_pending_terminal_events();

    let render = |selected: usize| {
        clear_menu_screen();
        raw_println!(
            "{}",
            theme::menu_title(&format!(
                "Choose a model to {}",
                if active { "download and activate" } else { "download" }
            ))
        );
        raw_println!("{}", theme::muted(format!("Backend {backend}")));
        if let Some(budget) = available_vram_mb {
            raw_println!(
                "{}",
                theme::muted(format!(
                    "Model budget {budget} MB VRAM · {}% contribution cap",
                    config.contribution_percent
                ))
            );
        }
        raw_println!("{}", theme::menu_rule());
        for (i, option) in options.iter().enumerate() {
            let is_selected = i == selected;
            let marker = theme::menu_marker(is_selected);
            let estimated = option
                .estimated_vram_mb
                .map(|value| format!("{value} MB VRAM"))
                .unwrap_or_else(|| "VRAM unknown".to_string());
            let backends = if option.backend_compatibility.is_empty() {
                "auto".to_string()
            } else {
                option
                    .backend_compatibility
                    .iter()
                    .map(|backend| backend.as_str())
                    .collect::<Vec<_>>()
                    .join("/")
            };
            raw_println!(
                "{marker} {}  {}",
                theme::menu_label(format!("{}. {}", i + 1, option.label), is_selected),
                theme::muted(format!("{} · {}", option.name, estimated))
            );
            raw_println!(
                "  {}",
                theme::muted(format!(
                    "provider {} · fit ok · backends {}",
                    option.source_kind, backends
                ))
            );
            raw_println!("  {}", theme::muted(&option.source_url));
        }
        raw_println!();
        raw_println!(
            "{}",
            theme::hint("Use ↑/↓ or Tab/Shift+Tab and Enter, or press a number · Ctrl-C cancels")
        );
        let _ = io::stdout().flush();
    };

    render(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) if handles_terminal_key(key.kind) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    eprintln!("cancelled");
                    std::process::exit(130);
                }
                KeyCode::Up | KeyCode::BackTab => {
                    selected = selected.saturating_sub(1);
                    render(selected);
                }
                KeyCode::Down | KeyCode::Tab => {
                    if selected + 1 < options.len() {
                        selected += 1;
                    }
                    render(selected);
                }
                KeyCode::Char(value) if value.is_ascii_digit() => {
                    if let Some(index) = value
                        .to_digit(10)
                        .and_then(|value| usize::try_from(value).ok())
                        .and_then(|value| value.checked_sub(1))
                    {
                        if index < options.len() {
                            break index;
                        }
                    }
                }
                KeyCode::Enter => break selected,
                KeyCode::Esc => break selected,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break selected,
        }
    };

    let _ = disable_raw_mode();
    options[result].clone()
}

fn model_name_for_command(config: &Config, name: Option<String>, active: bool) -> String {
    name.unwrap_or_else(|| prompt_official_model_selection(config, active).name)
}

fn apply_model_choice(config: &mut Config, choice: ModelChoice, active: bool) {
    match choice {
        ModelChoice::Model(model) => {
            ensure_effective_model_dir(config);
            let backend = resolved_backend(config);
            if let Err(error) = ensure_catalog_model_fits_machine(&model.name, backend, config) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            let result = if active {
                use_model(config, &model.name)
            } else {
                add_model(config, &model.name)
            };
            if let Err(error) = result {
                eprintln!("failed to cache model `{}`: {error}", model.name);
                std::process::exit(1);
            }
            print_model_event(
                "MODEL SELECTED",
                &model.name,
                "model recorded in local cache",
                Color::Cyan,
                config,
            );
        }
        ModelChoice::LocalGguf(path) => {
            let backend = resolved_backend(config);
            let available_vram_mb = model_vram_budget_mb(config, backend);
            let path = std::path::PathBuf::from(path);
            let record = match import_model(
                config,
                ImportModelOptions {
                    name: None,
                    path: &path,
                    active,
                    backend,
                    available_vram_mb,
                },
            ) {
                Ok(record) => record,
                Err(error) => {
                    eprintln!("failed to import model `{}`: {error}", path.display());
                    std::process::exit(1);
                }
            };
            if record.compatibility.as_deref() == Some("rejected") {
                eprintln!(
                    "model `{}` is not compatible: {}",
                    record.name,
                    record
                        .compatibility_reason
                        .as_deref()
                        .unwrap_or("rejected by compatibility check")
                );
                std::process::exit(1);
            }
            print_model_event(
                "MODEL IMPORTED",
                &record.name,
                record
                    .compatibility_reason
                    .as_deref()
                    .unwrap_or("local model metadata recorded"),
                Color::Cyan,
                config,
            );
        }
    }
}

/// The model a contributed cluster advertises, when one is recorded.
fn contributed_cluster_model(config: &Config) -> Option<String> {
    let cluster = config.contributed_cluster.as_ref()?;
    cluster
        .model
        .clone()
        .or_else(|| cluster.models.first().cloned())
}

/// The model this node actually serves: the contributed cluster's model when a
/// cluster was adopted, otherwise the locally cached active model.
fn effective_active_model(config: &Config) -> Option<String> {
    contributed_cluster_model(config).or_else(|| active_model_name(config))
}

fn cluster_choice_flag(contribute: bool, no_contribute: bool) -> Option<bool> {
    match (contribute, no_contribute) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// Asks how many concurrent jobs this node should accept from the control
/// plane.
///
/// Derived figures — a runtime's reported ceiling, or the memory ladder for a
/// MundusX-managed runtime — describe what a machine *could* hold, not what its
/// owner wants to give away. `None` means "leave it derived", which is the
/// right answer for most contributors and so is offered first.
fn prompt_max_jobs(context: &str, default_choice: Option<u32>) -> Option<u32> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return None;
    }

    let header = vec![
        "How many jobs may MundusX run at once on this node?".to_string(),
        format!("  {context}"),
        "  jobs beyond the limit wait rather than slowing the ones already running".to_string(),
        "-------------------------------------------------------------------------".to_string(),
    ];

    const CHOICES: [u32; 5] = [1, 2, 4, 8, 16];
    let mut options = vec![(
        "Automatic".to_string(),
        "let MundusX size this from the machine and runtime".to_string(),
    )];
    options.extend(CHOICES.iter().map(|jobs| {
        (
            format!("{jobs} job{}", if *jobs == 1 { "" } else { "s" }),
            match jobs {
                1 => "one at a time; the machine stays mostly yours".to_string(),
                16 => "heavy; most of a busy machine".to_string(),
                _ => String::new(),
            },
        )
    }));
    options.push(("custom".to_string(), "type an exact number".to_string()));

    let selected = default_choice
        .and_then(|jobs| CHOICES.iter().position(|choice| *choice == jobs))
        .map(|index| index + 1)
        .unwrap_or(0);

    match select_menu_option(
        &header,
        &options,
        "Use ↑/↓ or Tab/Shift+Tab and Enter",
        selected,
    ) {
        // "Automatic", or Esc: leave it derived.
        Some(0) | None => None,
        Some(index) if index <= CHOICES.len() => Some(CHOICES[index - 1]),
        Some(_) => read_custom_max_jobs(),
    }
}

/// Context line for the job-limit prompt, naming what the runtime reports.
fn cluster_capacity_context(cluster: &DetectedCluster) -> String {
    match cluster.max_concurrency {
        Some(reported) => format!(
            "the cluster at {} reports room for about {reported} concurrent requests",
            cluster.base_url
        ),
        None => format!(
            "the cluster at {} does not report its concurrency",
            cluster.base_url
        ),
    }
}

fn read_custom_max_jobs() -> Option<u32> {
    loop {
        print!("Concurrent jobs (1-64, blank to keep the default): ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return None;
        }
        let value = input.trim();
        if value.is_empty() {
            return None;
        }
        match value.parse::<u32>() {
            Ok(parsed) if (1..=64).contains(&parsed) => return Some(parsed),
            _ => eprintln!("enter a whole number from 1 through 64"),
        }
    }
}

fn contributed_cluster_from(
    cluster: &DetectedCluster,
    model: Option<String>,
) -> ContributedCluster {
    // Size travels with the advertised model so the node agent can classify its
    // capacity from the model rather than from host memory.
    let advertised = model
        .as_deref()
        .and_then(|name| cluster.models.iter().find(|entry| entry.name == name))
        .or_else(|| cluster.largest_model());

    ContributedCluster {
        kind: cluster.kind.as_str().to_string(),
        base_url: cluster.base_url.clone(),
        capacity_class: "server".to_string(),
        models: cluster.model_names(),
        model: model.or_else(|| cluster.primary_model().map(str::to_string)),
        model_params: advertised.and_then(|entry| entry.params),
        model_bytes: advertised.and_then(|entry| entry.bytes),
        model_capabilities: advertised
            .map(|entry| entry.capabilities.clone())
            .unwrap_or_default(),
        // What the endpoint serves wins over what the model was trained at: a
        // server started with a small `-c` cannot honour the trained length.
        model_context_tokens: cluster
            .served_context_tokens
            .or_else(|| advertised.and_then(|entry| entry.context_tokens)),
        memory_utilization: cluster.memory_utilization,
        max_concurrency: cluster.max_concurrency,
        max_num_seqs: cluster.max_num_seqs,
        kv_cache_tokens: cluster.kv_cache_tokens,
        supports_tool_calls: cluster.supports_tool_calls,
        adopted_at: Some(now_unix_seconds()),
    }
}

fn print_cluster_adopted_panel(cluster: &ContributedCluster) {
    let body = vec![
        format!("runtime: {}", cluster.kind),
        format!("endpoint: {}", cluster.base_url),
        format!(
            "model: {}",
            cluster.model.as_deref().unwrap_or("none advertised")
        ),
        format!("models available: {}", cluster.models.len()),
        "MundusX will serve work from this cluster instead of downloading its own model"
            .to_string(),
        "run `opengpu cluster forget` to stop contributing it".to_string(),
    ];
    print_retro_panel(
        "CLUSTER CONTRIBUTED",
        "existing local cluster joined",
        &body,
        Color::Green,
    );
}

fn discover_clusters(cluster_url: Option<&str>) -> Vec<DetectedCluster> {
    match cluster_url {
        Some(url) => match cluster::normalize_base_url(url) {
            Some(_) => cluster::probe_cluster(url).into_iter().collect(),
            None => {
                eprintln!("clusterError: `{url}` must be a full http:// or https:// URL");
                std::process::exit(2);
            }
        },
        None => cluster::detect_running_clusters(),
    }
}

/// Probes for running clusters during setup, honoring the detection opt-out.
fn detect_clusters_for_setup(cluster_url: Option<&str>) -> Vec<DetectedCluster> {
    if cluster_url.is_none() && cluster::detection_disabled() {
        return Vec::new();
    }
    discover_clusters(cluster_url)
}

/// Records a cluster as contributed and prints the confirmation panel.
fn contribute_detected_cluster(
    config: &mut Config,
    cluster: &DetectedCluster,
    max_jobs: Option<u32>,
) {
    let contributed = contributed_cluster_from(cluster, None);
    if let Some(jobs) = max_jobs.filter(|value| *value > 0) {
        config.max_jobs = Some(jobs);
    }
    print_cluster_adopted_panel(&contributed);
    config.contributed_cluster = Some(contributed);
    config.cluster_prompt_declined = false;
}

/// Detects a running local cluster and asks whether to contribute it.
///
/// Returns true when the node ends up contributing a cluster, which lets the
/// caller skip MundusX model provisioning entirely.
fn maybe_contribute_running_cluster(
    config: &mut Config,
    forced: Option<bool>,
    cluster_url: Option<&str>,
    pre_detected: Option<&[DetectedCluster]>,
    // True when the caller already offered the cluster list itself, so this
    // function must not open a second picker.
    suppress_prompt: bool,
    // Set by `--max-jobs`, which answers the prompt without asking.
    forced_max_jobs: Option<u32>,
) -> bool {
    let explicit_probe = cluster_url.is_some();
    if pre_detected.is_none() && cluster::detection_disabled() && !explicit_probe {
        return config.contributed_cluster.is_some();
    }

    let probed;
    let clusters = match pre_detected {
        Some(clusters) => clusters,
        None => {
            probed = discover_clusters(cluster_url);
            &probed
        }
    };
    let candidate = cluster::preferred_cluster(clusters).cloned();
    let servable = candidate
        .as_ref()
        .map(DetectedCluster::is_servable)
        .unwrap_or(false);

    if let Some(idle) = candidate.as_ref().filter(|entry| !entry.is_servable()) {
        println!("clusterDetected: {}", idle.summary());
        println!(
            "clusterHint: load a model in that runtime and re-run `opengpu cluster scan` to contribute it"
        );
    }

    let decision = cluster::cluster_prompt_decision(
        servable,
        config.contributed_cluster.is_some(),
        config.cluster_prompt_declined,
        forced,
        !suppress_prompt && io::stdin().is_terminal() && io::stdout().is_terminal(),
    );

    let contribute = match decision {
        ClusterPromptDecision::Skip(reason) => {
            match config.contributed_cluster.as_ref() {
                Some(cluster) => println!(
                    "clusterContributed: {} at {}",
                    cluster.kind, cluster.base_url
                ),
                None if servable => println!("clusterSkipped: {reason}"),
                None => {}
            }
            return config.contributed_cluster.is_some();
        }
        ClusterPromptDecision::AutoContribute => true,
        ClusterPromptDecision::AutoDecline => false,
        ClusterPromptDecision::Ask => {
            let ranked = cluster::servable_clusters_by_size(clusters);
            match prompt_cluster_pick(&ranked) {
                ClusterPickOutcome::Picked(index) => {
                    let chosen = ranked[index];
                    let jobs = forced_max_jobs
                        .or_else(|| prompt_max_jobs(&cluster_capacity_context(chosen), None));
                    contribute_detected_cluster(config, chosen, jobs);
                    return true;
                }
                ClusterPickOutcome::Declined => false,
                // Esc: leave the question open so the next run asks again.
                ClusterPickOutcome::Skipped => {
                    println!("clusterSkipped: no answer recorded; you will be asked again");
                    println!("clusterHint: run `opengpu cluster use <url>` to contribute one now");
                    return false;
                }
            }
        }
    };

    let Some(cluster) = candidate else {
        return config.contributed_cluster.is_some();
    };

    if !contribute {
        config.contributed_cluster = None;
        config.cluster_prompt_declined = true;
        println!("clusterDeclined: no running cluster will be contributed");
        println!("clusterHint: run `opengpu cluster use <url>` if you change your mind");
        return false;
    }

    let jobs =
        forced_max_jobs.or_else(|| prompt_max_jobs(&cluster_capacity_context(&cluster), None));
    contribute_detected_cluster(config, &cluster, jobs);
    true
}

fn run_cluster_scan(cluster_url: Option<&str>, json: bool) {
    let config = current_config_or_default();
    let clusters = discover_clusters(cluster_url);

    if json {
        let payload = serde_json::json!({
            "detection_disabled": cluster::detection_disabled() && cluster_url.is_none(),
            "contributed": config.contributed_cluster,
            "detected": clusters
                .iter()
                .map(|entry| serde_json::json!({
                    "kind": entry.kind.as_str(),
                    "base_url": entry.base_url,
                    "models": entry.models.iter().map(|model| serde_json::json!({
                        "name": model.name,
                        "params": model.params,
                        "bytes": model.bytes,
                    })).collect::<Vec<_>>(),
                    "largest_model": entry.primary_model(),
                    "servable": entry.is_servable(),
                }))
                .collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).expect("cluster scan payload")
        );
        return;
    }

    theme::section("Cluster scan");
    match config.contributed_cluster.as_ref() {
        Some(cluster) => {
            theme::field(
                "contributed",
                format!("{} at {}", cluster.kind, cluster.base_url),
            );
            theme::field(
                "contributedModel",
                cluster.model.as_deref().unwrap_or("none advertised"),
            );
        }
        None => theme::field("contributed", "none"),
    }

    if clusters.is_empty() {
        theme::field("detected", "none");
        if cluster::detection_disabled() && cluster_url.is_none() {
            theme::note("detection is disabled by OPENGPU_SKIP_CLUSTER_DETECT=1");
        }
        return;
    }

    for entry in cluster::servable_clusters_by_size(&clusters) {
        theme::field(entry.kind.as_str(), entry.summary());
    }
    for entry in clusters.iter().filter(|entry| !entry.is_servable()) {
        theme::field(entry.kind.as_str(), entry.summary());
    }
    theme::note("listed biggest model first; run `opengpu cluster use <url>` to contribute one");
}

fn run_cluster_use(url: &str, model: Option<String>, max_jobs: Option<u32>) {
    let Some(base_url) = cluster::normalize_base_url(url) else {
        eprintln!("clusterError: `{url}` must be a full http:// or https:// URL");
        std::process::exit(2);
    };

    let Some(detected) = cluster::probe_cluster(&base_url) else {
        eprintln!("clusterError: no LLM cluster answered at {base_url}");
        eprintln!(
            "clusterHint: confirm the runtime is serving and exposes /v1/models or /api/tags"
        );
        std::process::exit(1);
    };

    if let Some(requested) = model.as_ref() {
        if !detected.models.is_empty()
            && !detected.models.iter().any(|entry| &entry.name == requested)
        {
            eprintln!("clusterError: `{requested}` is not served by {base_url}");
            eprintln!("clusterModels: {}", detected.model_names().join(", "));
            std::process::exit(1);
        }
    }

    if !detected.is_servable() && model.is_none() {
        eprintln!("clusterError: {base_url} is running but advertises no model");
        eprintln!("clusterHint: load a model in that runtime, or pass `--model <name>`");
        std::process::exit(1);
    }

    let mut config = current_config_or_default();
    let contributed = contributed_cluster_from(&detected, model);
    if let Some(jobs) = max_jobs.filter(|value| *value > 0) {
        config.max_jobs = Some(jobs);
    }
    print_cluster_adopted_panel(&contributed);
    config.contributed_cluster = Some(contributed);
    config.cluster_prompt_declined = false;

    if let Err(error) = save_config(&config) {
        eprintln!("failed to save contributed cluster: {error}");
        std::process::exit(1);
    }
}

fn run_cluster_forget() {
    let mut config = current_config_or_default();
    match config.contributed_cluster.take() {
        Some(cluster) => println!("clusterForgotten: {} at {}", cluster.kind, cluster.base_url),
        None => println!("clusterForgotten: none was contributed"),
    }
    config.cluster_prompt_declined = false;

    if let Err(error) = save_config(&config) {
        eprintln!("failed to clear contributed cluster: {error}");
        std::process::exit(1);
    }

    if active_model_name(&config).is_none() {
        println!("clusterHint: run `opengpu start` to choose a MundusX model for this node");
    }
}

fn now_unix_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

/// Everything `opengpu start` needs that install can verify without a network
/// call. Reported at the end of install so a machine that cannot start says so
/// now instead of failing later.
///
/// Pure so the reporting can be tested without touching the filesystem.
fn start_preflight_blockers(
    config: &Config,
    identity_ready: bool,
    node_agent_installed: bool,
    cluster_serving_model: Option<bool>,
) -> Vec<String> {
    let mut blockers = Vec::new();

    if !identity_ready {
        blockers.push(
            "secure device identity is missing; `opengpu start` creates it automatically"
                .to_string(),
        );
    }
    if !node_agent_installed {
        blockers.push(format!(
            "`{}` is not installed beside `opengpu`",
            companion_node_agent_name()
        ));
    }
    if config.contribution_percent == 0 {
        blockers.push("contribution cap is unset; run `opengpu cap`".to_string());
    }

    match config.contributed_cluster.as_ref() {
        Some(cluster) => match cluster_serving_model {
            Some(true) => {}
            Some(false) => blockers.push(format!(
                "contributed cluster at {} is not serving `{}`",
                cluster.base_url,
                cluster.model.as_deref().unwrap_or("its advertised model")
            )),
            None => blockers.push(format!(
                "contributed cluster at {} is not answering",
                cluster.base_url
            )),
        },
        None => {
            if active_model_name(config).is_none() {
                blockers.push("no active model is selected".to_string());
            }
        }
    }

    blockers
}

/// Re-probes a contributed cluster: `None` when the endpoint is silent, and
/// `Some(false)` when it answers but no longer serves the advertised model.
fn contributed_cluster_serving_model(config: &Config) -> Option<bool> {
    let cluster = config.contributed_cluster.as_ref()?;
    let detected = cluster::probe_cluster(&cluster.base_url)?;
    let Some(expected) = cluster.model.as_deref() else {
        return Some(detected.is_servable());
    };
    Some(detected.models.iter().any(|model| model.name == expected))
}

fn print_start_preflight(config: &Config) {
    let cluster_serving = contributed_cluster_serving_model(config);
    let blockers = start_preflight_blockers(
        config,
        identity_ready(),
        resolve_node_agent_executable().is_absolute(),
        cluster_serving,
    );

    if blockers.is_empty() {
        theme::section("Ready to contribute");
        theme::field("status", theme::status("ready"));
        if config.contributed_cluster.is_some() {
            theme::note(
                "This node serves work from the contributed cluster; the control plane must admit the `contributed-cluster` runtime mode",
            );
        }
        return;
    }

    theme::section("Not ready to start");
    theme::field("status", theme::status("blocked"));
    for blocker in &blockers {
        theme::warn(blocker);
    }
    theme::note(
        "Fix any other items above, then run `opengpu start`; it creates a missing identity automatically",
    );
}

fn run_init() -> Config {
    let (identity, created, identity_path) = match ensure_identity() {
        Ok(result) => result,
        Err(error) => {
            eprintln!("failed to initialize identity: {error}");
            std::process::exit(1);
        }
    };

    let profile = detect_machine_profile();
    let mut config = config_from_identity(&identity);
    config.backend_preference = profile.backend;

    match save_config(&config) {
        Ok(path) => {
            if created {
                println!("generated device identity at {}", identity_path.display());
            } else {
                println!("reused device identity at {}", identity_path.display());
            }
            println!("initialized {}", path.display());
        }
        Err(error) => {
            eprintln!("failed to initialize config: {error}");
            std::process::exit(1);
        }
    }

    config
}

fn run_install(
    public: bool,
    private: bool,
    control_plane_url: Option<String>,
    cap_percent: Option<u8>,
    cluster_choice: Option<bool>,
    cluster_url: Option<String>,
    max_jobs: Option<u32>,
) {
    theme::banner(
        "Set up this machine for OpenGPU",
        "Private compute. Your limits. The MundusX network.",
    );
    let profile = detect_machine_profile();
    // A previous or interrupted setup can leave config.json behind without an
    // identity. Always create/validate secure identity independently, then make
    // the saved config refer to it.
    let identity = match ensure_identity() {
        Ok((identity, _, _)) => identity,
        Err(error) => {
            eprintln!("failed to initialize identity: {error}");
            std::process::exit(1);
        }
    };
    let mut config = if config_exists() {
        current_config_or_default()
    } else {
        Config::default()
    };
    sync_config_identity(&mut config, &identity);
    config.backend_preference = profile.backend;
    let detected = resolved_backend(&config);

    // The contributor's job limit is a property of the node, not of whichever
    // runtime it ends up serving, so record it before anything is detected.
    if let Some(jobs) = max_jobs.filter(|value| *value > 0) {
        config.max_jobs = Some(jobs);
    }

    // Probe up front so the cluster step after the cap has results ready.
    let detected_clusters = detect_clusters_for_setup(cluster_url.as_deref());

    config.control_plane_url =
        resolve_install_control_plane_url(public, private, control_plane_url);
    if let Err(error) = save_config(&config) {
        eprintln!("failed to save control-plane URL: {error}");
        std::process::exit(1);
    }
    theme::field("control plane", &config.control_plane_url);

    let ranked_clusters = cluster::servable_clusters_by_size(&detected_clusters);
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut contributed_from_menu = false;

    let selected_cap = if let Some(value) = cap_percent {
        match normalize_contribution_percent(u16::from(value)) {
            Ok(value) => Some(value),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    } else if interactive {
        theme::note(format!(
            "Choose how much of this {} machine MundusX may use",
            detected.as_str()
        ));
        theme::note(contribution_semantics(detected));
        let default_percent = default_contribution_percent(detected);
        // The cluster list is reached from this menu, and Esc or "None" inside it
        // comes back here so the cap can still be chosen.
        loop {
            match prompt_contribution_percent(default_percent, ranked_clusters.len()) {
                PromptOutcome::Selected(value) => break Some(value),
                PromptOutcome::Cancelled => break None,
                PromptOutcome::UseCluster => match prompt_cluster_pick(&ranked_clusters) {
                    ClusterPickOutcome::Picked(index) => {
                        let chosen = ranked_clusters[index];
                        let jobs = max_jobs
                            .or_else(|| prompt_max_jobs(&cluster_capacity_context(chosen), None));
                        contribute_detected_cluster(&mut config, chosen, jobs);
                        contributed_from_menu = true;
                        // A contributed cluster is not gated by the cap, but the
                        // node still needs one saved to pass local policy.
                        theme::note(format!(
                            "The cap does not gate a contributed cluster; saved {default_percent}% for local policy"
                        ));
                        break Some(default_percent);
                    }
                    ClusterPickOutcome::Declined | ClusterPickOutcome::Skipped => {}
                },
            }
        }
    } else if config.contribution_percent == 0 {
        Some(default_contribution_percent(detected))
    } else {
        None
    };

    if let Some(value) = selected_cap {
        config.contribution_percent = value;
    }

    let contributing_cluster = contributed_from_menu
        || maybe_contribute_running_cluster(
            &mut config,
            cluster_choice,
            cluster_url.as_deref(),
            Some(&detected_clusters),
            // Interactive installs ask through the contribution level menu.
            !interactive,
            max_jobs,
        );

    if should_prompt_model_selection(&config) {
        let backend = resolved_backend(&config);
        let choice = prompt_model_selection(&config, backend);
        apply_model_choice(&mut config, choice, true);
    }

    if !contributing_cluster {
        configure_macos_runtime(&mut config);

        // A contributor running a MundusX-managed runtime gets the same say
        // over concurrency as one contributing a cluster.
        if config.max_jobs.is_none() && interactive {
            let context = format!(
                "MundusX runs the model here, sized from this machine's memory and a {}% cap",
                config.contribution_percent
            );
            if let Some(jobs) = prompt_max_jobs(&context, None) {
                config.max_jobs = Some(jobs);
            }
        }
    }

    match save_config(&config) {
        Ok(path) => {
            let body = vec![
                format!("machine: {} {}", profile.os, profile.arch),
                format!("install profile: {}", profile.install_profile),
                format!(
                    "gpu: {}",
                    profile.cuda_gpu_name.as_deref().unwrap_or("none detected")
                ),
                format!(
                    "cuda vram: {}",
                    profile
                        .cuda_vram_mb
                        .map(|value| format!("{value} MB"))
                        .unwrap_or_else(|| "none detected".to_string())
                ),
                format!("control plane: {}", config.control_plane_url),
                format!(
                    "backend: {}",
                    if config.backend_preference.is_auto() {
                        detected.as_str()
                    } else {
                        config.backend_preference.as_str()
                    }
                ),
                format!(
                    "runtime preference: {}",
                    config
                        .runtime_preference
                        .as_deref()
                        .unwrap_or("platform default")
                ),
                format!(
                    "fallback runtime: {}",
                    config.fallback_runtime.as_deref().unwrap_or("none")
                ),
                format!(
                    "contribution cap: {}",
                    if config.contribution_percent == 0 {
                        "unset".to_string()
                    } else {
                        format!("{}%", config.contribution_percent)
                    }
                ),
                format!(
                    "contributed cluster: {}",
                    config
                        .contributed_cluster
                        .as_ref()
                        .map(|cluster| format!("{} at {}", cluster.kind, cluster.base_url))
                        .unwrap_or_else(|| "none".to_string())
                ),
                format!(
                    "active model: {}",
                    effective_active_model(&config).unwrap_or_else(|| "none".to_string())
                ),
                format!("config: {}", path.display()),
                "next step: run `opengpu start`".to_string(),
            ];
            print_retro_panel(
                "INSTALL COMPLETE",
                "machine setup saved",
                &body,
                Color::Green,
            );
            print_start_preflight(&config);
        }
        Err(error) => {
            eprintln!("failed to save install setup: {error}");
            std::process::exit(1);
        }
    }
}

/// What a re-probe found about the cluster this node already contributes.
#[derive(Clone, Debug, PartialEq)]
enum ContributedClusterCheck {
    /// No cluster is recorded, so there is nothing to verify.
    NotContributed,
    /// Still answering with the model this node advertises.
    Unchanged,
    /// Answering, but the advertised model is gone. Carries what it serves now.
    ModelChanged {
        recorded: String,
        detected: Box<DetectedCluster>,
    },
    /// The endpoint did not answer at all.
    Unreachable { base_url: String },
}

/// Re-probes the recorded cluster so `opengpu start` never begins with a node
/// pointed at a model that is no longer there.
///
/// Pure given the probe result, so the decision can be tested without a socket.
fn classify_contributed_cluster(
    recorded: Option<&ContributedCluster>,
    probe: Option<DetectedCluster>,
) -> ContributedClusterCheck {
    let Some(recorded) = recorded else {
        return ContributedClusterCheck::NotContributed;
    };
    let Some(detected) = probe else {
        return ContributedClusterCheck::Unreachable {
            base_url: recorded.base_url.clone(),
        };
    };

    let advertised = recorded
        .model
        .clone()
        .or_else(|| recorded.models.first().cloned())
        .unwrap_or_default();
    if detected.models.iter().any(|model| model.name == advertised) {
        return ContributedClusterCheck::Unchanged;
    }

    ContributedClusterCheck::ModelChanged {
        recorded: advertised,
        detected: Box::new(detected),
    }
}

/// Verifies the recorded cluster and, when something changed, says so and asks
/// what to do. Returns false when the node should not start contributing.
fn verify_contributed_cluster(config: &mut Config) -> bool {
    let Some(recorded) = config.contributed_cluster.clone() else {
        return true;
    };

    let probe = cluster::probe_cluster(&recorded.base_url);
    let check = classify_contributed_cluster(Some(&recorded), probe);
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();

    match check {
        ContributedClusterCheck::NotContributed | ContributedClusterCheck::Unchanged => {
            // The served context can move without the model changing, so keep
            // the advertised value honest rather than trusting the snapshot.
            if let Some(detected) = cluster::probe_cluster(&recorded.base_url) {
                refresh_contributed_cluster_context(config, &detected);
            }
            true
        }
        ContributedClusterCheck::ModelChanged {
            recorded: was,
            detected,
        } => {
            let now = detected.primary_model().unwrap_or("nothing").to_string();
            println!(
                "clusterChanged: {} no longer serves `{was}`",
                recorded.base_url
            );
            println!("clusterNowServing: {} ({})", now, detected.kind.label());
            if !interactive {
                println!(
                    "clusterHint: run `opengpu cluster use {}` to serve `{now}`, or `opengpu cluster forget` to stop",
                    recorded.base_url
                );
                return false;
            }
            prompt_contributed_cluster_change(config, &detected, &was)
        }
        ContributedClusterCheck::Unreachable { base_url } => {
            println!("clusterUnreachable: {base_url} is not answering");
            if !interactive {
                println!(
                    "clusterHint: start it again, or run `opengpu cluster forget` to stop contributing it"
                );
                return false;
            }
            prompt_contributed_cluster_unreachable(config, &base_url)
        }
    }
}

fn refresh_contributed_cluster_context(config: &mut Config, detected: &DetectedCluster) {
    let Some(cluster) = config.contributed_cluster.as_mut() else {
        return;
    };
    let served = detected.served_context_tokens.or_else(|| {
        detected
            .models
            .iter()
            .find(|model| Some(model.name.as_str()) == cluster.model.as_deref())
            .and_then(|model| model.context_tokens)
    });
    if served.is_some() && served != cluster.model_context_tokens {
        println!(
            "clusterContext: served window is now {} tokens",
            served.unwrap_or_default()
        );
        cluster.model_context_tokens = served;
    }
    if let Some(max_num_seqs) = detected.max_num_seqs {
        if cluster.max_num_seqs != Some(max_num_seqs) {
            println!("clusterConcurrency: max_num_seqs is now {max_num_seqs}");
            cluster.max_num_seqs = Some(max_num_seqs);
        }
    }
    if let Some(max_concurrency) = detected.max_concurrency {
        cluster.max_concurrency = Some(max_concurrency);
    }
    if let Some(kv_cache_tokens) = detected.kv_cache_tokens {
        cluster.kv_cache_tokens = Some(kv_cache_tokens);
    }
    if let Some(memory_utilization) = detected.memory_utilization {
        cluster.memory_utilization = Some(memory_utilization);
    }
}

/// The cluster answered but serves something else now.
fn prompt_contributed_cluster_change(
    config: &mut Config,
    detected: &DetectedCluster,
    was: &str,
) -> bool {
    // Switching models does not change how much of the node is on offer.
    let existing_max_jobs = config.max_jobs;
    let now = detected.primary_model().unwrap_or("nothing").to_string();
    let header = vec![
        format!("The contributed cluster changed at {}", detected.base_url),
        format!("  recorded: {was}"),
        format!("  serving:  {} ({})", now, detected.kind.label()),
        "-----------------------------------------".to_string(),
    ];
    let options = vec![
        (
            format!("Yes, serve {now}"),
            "update this node to the model the cluster runs now".to_string(),
        ),
        (
            "No, stop contributing this cluster".to_string(),
            "forget it and pick a MundusX model instead".to_string(),
        ),
        (
            "No, leave it as it is".to_string(),
            "start anyway; jobs for the old model will be refused".to_string(),
        ),
    ];

    match select_menu_option(&header, &options, "Use ↑/↓ or Tab/Shift+Tab and Enter", 0) {
        Some(0) => {
            contribute_detected_cluster(config, detected, existing_max_jobs);
            true
        }
        Some(1) => {
            config.contributed_cluster = None;
            config.cluster_prompt_declined = false;
            println!("clusterForgotten: {}", detected.base_url);
            true
        }
        // Enter on "leave it", or Esc, changes nothing.
        _ => true,
    }
}

/// The cluster did not answer at all.
fn prompt_contributed_cluster_unreachable(config: &mut Config, base_url: &str) -> bool {
    let existing_max_jobs = config.max_jobs;
    let others: Vec<DetectedCluster> = cluster::detect_running_clusters()
        .into_iter()
        .filter(|entry| entry.base_url != base_url && entry.is_servable())
        .collect();
    let ranked = cluster::servable_clusters_by_size(&others);

    let header = vec![
        format!("The contributed cluster at {base_url} is not answering."),
        "-----------------------------------------".to_string(),
    ];
    let mut options = vec![(
        "Keep it and start anyway".to_string(),
        "the node stays unready until the cluster returns".to_string(),
    )];
    if !ranked.is_empty() {
        options.push((
            format!("Pick a different cluster ({} running)", ranked.len()),
            "contribute one of the other endpoints instead".to_string(),
        ));
    }
    options.push((
        "Stop contributing it".to_string(),
        "forget it and pick a MundusX model instead".to_string(),
    ));

    let pick_index = if ranked.is_empty() { usize::MAX } else { 1 };
    let forget_index = options.len() - 1;

    match select_menu_option(&header, &options, "Use ↑/↓ or Tab/Shift+Tab and Enter", 0) {
        Some(index) if index == pick_index => match prompt_cluster_pick(&ranked) {
            ClusterPickOutcome::Picked(chosen) => {
                contribute_detected_cluster(config, ranked[chosen], existing_max_jobs);
                true
            }
            ClusterPickOutcome::Declined => {
                config.contributed_cluster = None;
                config.cluster_prompt_declined = true;
                println!("clusterForgotten: {base_url}");
                true
            }
            ClusterPickOutcome::Skipped => true,
        },
        Some(index) if index == forget_index => {
            config.contributed_cluster = None;
            config.cluster_prompt_declined = false;
            println!("clusterForgotten: {base_url}");
            println!("clusterHint: `opengpu start` will now ask for a MundusX model");
            true
        }
        // "Keep it and start anyway", or Esc.
        _ => {
            println!("clusterKept: {base_url}; the node will stay unready until it answers");
            true
        }
    }
}

fn run_start_or_connect(
    mode: AgentLaunchMode,
    cluster_choice: Option<bool>,
    cluster_url: Option<String>,
    max_jobs: Option<u32>,
) {
    // auto-init on first run
    if !config_exists() {
        run_init();
    }

    let mut config = current_config_or_default();
    let identity_ready = match load_or_create_identity() {
        Ok((identity, _, _)) => {
            config.device_id = device_id_for_identity(&identity);
            config.public_key_fingerprint = Some(identity.fingerprint);
            true
        }
        Err(error) => {
            theme::error(format!("Failed to load the secure device identity: {error}"));
            false
        }
    };
    if config.contribution_percent == 0 && io::stdin().is_terminal() && io::stdout().is_terminal() {
        let detected = resolved_backend(&config);
        theme::note(format!(
            "Choose how much of this {} machine MundusX may use",
            detected.as_str()
        ));
        theme::note(contribution_semantics(detected));
        match prompt_contribution_percent(default_contribution_percent(detected), 0) {
            PromptOutcome::Selected(value) => {
                config.contribution_percent = value;
            }
            // The cluster row is not offered here; `start` asks separately below.
            PromptOutcome::UseCluster | PromptOutcome::Cancelled => {
                theme::note("Run `opengpu cap` before starting contribution");
            }
        }
    }
    if let Some(jobs) = max_jobs.filter(|value| *value > 0) {
        config.max_jobs = Some(jobs);
    }

    // A recorded cluster can disappear or swap models between runs, so check it
    // before this node advertises anything about it.
    if !verify_contributed_cluster(&mut config) {
        let _ = save_config(&config);
        std::process::exit(1);
    }

    maybe_contribute_running_cluster(
        &mut config,
        cluster_choice,
        cluster_url.as_deref(),
        None,
        false,
        max_jobs,
    );

    if should_prompt_model_selection(&config) {
        let backend = resolved_backend(&config);
        let choice = prompt_model_selection(&config, backend);
        apply_model_choice(&mut config, choice, true);
    }
    if let Some(active_model) = active_model_name(&config) {
        let backend = resolved_backend(&config);
        if let Err(error) = ensure_catalog_model_fits_machine(&active_model, backend, &config) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        if let Err(error) = use_model(&mut config, &active_model) {
            eprintln!("failed to refresh active model `{active_model}`: {error}");
            std::process::exit(1);
        }
    }
    config.connected = identity_ready;
    config.paused = !identity_ready;

    match save_config(&config) {
        Ok(_) => {
            print_startup_summary(&config, &resolved_config_path());
            theme::note(contribution_semantics(config.backend_preference));
            if config.contribution_percent == 0 {
                theme::note("Run `opengpu cap` to choose the contribution budget");
            }
            if !identity_ready {
                theme::warn("Secure device identity is unavailable; this node is not online yet");
            }
            if !config.onboarding_completed {
                print_onboarding_checklist(&config, &resolved_config_path(), false);
                theme::note(
                    "Run `opengpu onboarding --complete` after reviewing the checklist",
                );
            }
            if identity_ready {
                if let Err(error) = launch_node_agent(mode) {
                    theme::error(error);
                    theme::error(
                        "agentHint: ensure `opengpu-node-agent` is installed beside `opengpu`, or run `opengpu start --background` to use daemon mode"
                    );
                    std::process::exit(1);
                }
            }
        }
        Err(error) => {
            eprintln!("failed to save config: {error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let cli = Cli::parse();
    theme::configure(cli.theme);

    match cli.command {
        Commands::Install {
            public,
            private,
            control_plane_url,
            cap_percent,
            contribute_cluster,
            no_contribute_cluster,
            cluster_url,
            max_jobs,
        } => run_install(
            public,
            private,
            control_plane_url,
            cap_percent,
            cluster_choice_flag(contribute_cluster, no_contribute_cluster),
            cluster_url,
            max_jobs,
        ),
        Commands::Start {
            background,
            debug,
            contribute_cluster,
            no_contribute_cluster,
            cluster_url,
            max_jobs,
        } => {
            let mode = if background {
                AgentLaunchMode::Background
            } else if debug {
                AgentLaunchMode::ForegroundDebug
            } else {
                AgentLaunchMode::Foreground
            };
            run_start_or_connect(
                mode,
                cluster_choice_flag(contribute_cluster, no_contribute_cluster),
                cluster_url,
                max_jobs,
            )
        }
        Commands::Connect => run_start_or_connect(AgentLaunchMode::Background, None, None, None),
        Commands::Pause => {
            if !config_exists() {
                eprintln!("not connected");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            if !config.connected {
                eprintln!("not connected");
                std::process::exit(1);
            }

            config.paused = true;
            match save_config(&config) {
                Ok(_) => {
                    println!("connected: yes");
                    println!("paused: yes");
                    println!("pauseState: draining");
                    println!("pauseHint: the node agent will stop claiming jobs and cool the warm runtime");
                }
                Err(error) => {
                    eprintln!("failed to save pause state: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Resume => run_start_or_connect(AgentLaunchMode::Background, None, None, None),
        Commands::Login { token } => {
            let mut config = current_config_or_default();
            let token = match token {
                Some(token) => token.trim().to_string(),
                None => read_operator_token_from_prompt().unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                }),
            };

            if token.is_empty() {
                eprintln!("operator token cannot be empty");
                std::process::exit(1);
            }

            #[cfg(windows)]
            {
                if let Err(error) = auth_token::store_operator_token(&token) {
                    eprintln!("failed to store protected auth token: {error}");
                    std::process::exit(1);
                }
                config.auth_token = None;
            }
            #[cfg(not(windows))]
            {
                config.auth_token = Some(token);
            }
            match save_config(&config) {
                Ok(path) => {
                    #[cfg(windows)]
                    let _ = &path;
                    println!("authenticated: yes");
                    #[cfg(windows)]
                    println!(
                        "authTokenPath: {}",
                        auth_token::protected_token_path().display()
                    );
                    #[cfg(not(windows))]
                    println!("authTokenPath: {}", path.display());
                }
                Err(error) => {
                    eprintln!("failed to save auth token: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Logout => {
            let mut config = current_config_or_default();
            if let Err(error) = auth_token::clear_operator_token() {
                eprintln!("failed to clear protected auth token: {error}");
                std::process::exit(1);
            }
            config.auth_token = None;
            match save_config(&config) {
                Ok(_) => {
                    println!("authenticated: no");
                }
                Err(error) => {
                    eprintln!("failed to clear auth token: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Onboarding { complete, reset } => {
            let mut config = current_config_or_default();
            if complete && reset {
                eprintln!("choose either --complete or --reset, not both");
                std::process::exit(1);
            }

            if reset {
                config.onboarding_completed = false;
            } else if complete {
                config.onboarding_completed = true;
            }

            if let Err(error) = save_config(&config) {
                eprintln!("failed to save onboarding state: {error}");
                std::process::exit(1);
            }

            let active_model = effective_active_model(&config);
            let identity_ready = identity_ready();
            let power = probe_power_state();
            let policy_allowed =
                policy_allowed(&config, &power, active_model.as_deref(), identity_ready);
            print_onboarding_checklist(
                &config,
                &resolved_config_path(),
                config.onboarding_completed,
            );
            println!(
                "onboardingCompleted: {}",
                if config.onboarding_completed {
                    "yes"
                } else {
                    "no"
                }
            );
            if config.onboarding_completed {
                println!(
                    "onboardingHint: {}",
                    onboarding_completion_hint(
                        &config,
                        identity_ready,
                        active_model.as_deref(),
                        policy_allowed
                    )
                );
            } else {
                println!("onboardingHint: rerun with --complete once the checklist looks good");
            }
        }
        Commands::Cap {
            value,
            percent,
            reset,
        } => {
            if reset && (value.is_some() || percent.is_some()) {
                eprintln!("choose either a cap percent or --reset, not both");
                std::process::exit(1);
            }
            if value.is_some() && percent.is_some() {
                eprintln!("choose either positional percent or --percent, not both");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            let selected = if reset {
                config.contribution_percent = 0;
                None
            } else if let Some(value) = value.or(percent) {
                let value = match normalize_contribution_percent(u16::from(value)) {
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                };
                config.contribution_percent = value;
                Some(value)
            } else {
                match prompt_contribution_percent(
                    if config.contribution_percent == 0 {
                        30
                    } else {
                        config.contribution_percent
                    },
                    0,
                ) {
                    PromptOutcome::Selected(value) => {
                        config.contribution_percent = value;
                        Some(value)
                    }
                    // `opengpu cap` never offers the cluster row.
                    PromptOutcome::UseCluster | PromptOutcome::Cancelled => {
                        eprintln!("cancelled");
                        std::process::exit(130);
                    }
                }
            };

            if let Err(error) = save_config(&config) {
                eprintln!("failed to save contribution cap: {error}");
                std::process::exit(1);
            }

            print_contribution_cap(&config, selected, !reset && selected.is_some());
            println!(
                "contributionPercent: {}",
                if config.contribution_percent == 0 {
                    "unset".to_string()
                } else {
                    format!("{}%", config.contribution_percent)
                }
            );
            println!(
                "capHint: {}",
                if config.contribution_percent == 0 {
                    "rerun `opengpu cap` to choose one".to_string()
                } else {
                    "run `opengpu start` to bring the node online".to_string()
                }
            );
        }
        Commands::Disconnect => {
            if !config_exists() {
                eprintln!("not connected");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            config.connected = false;
            config.paused = true;

            match save_config(&config) {
                Ok(_) => {
                    println!("disconnected {}", config.device_id);
                    println!("connected: no");
                    println!("paused: yes");
                    match disconnect_node_agent() {
                        Ok(Some(pid)) => {
                            println!("agent: stopped");
                            println!("agentPid: {pid}");
                        }
                        Ok(None) => println!("agent: not running"),
                        Err(error) => {
                            theme::error(format!("agentStop: {error}"));
                            std::process::exit(1);
                        }
                    }
                }
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Status { json } => {
            let config = current_config_or_default();
            let preferred_backend = resolved_backend(&config);
            let power = probe_power_state();
            let active_model = effective_active_model(&config);
            let identity_ready = identity_ready();
            let policy_allowed =
                policy_allowed(&config, &power, active_model.as_deref(), identity_ready);
            let readiness =
                local_readiness(&config, &power, active_model.as_deref(), identity_ready);
            let provider_count =
                provider_count(&config, &power, active_model.as_deref(), identity_ready);

            if json {
                let payload = serde_json::json!({
                    "config": config,
                    "detected_backend": preferred_backend,
                    "provider_count": provider_count,
                    "routing_mode": "local-only",
                    "selected_provider": if provider_count == 1 { "self" } else { "none" },
                    "power_state": {
                        "source": power.source,
                        "on_battery": power.on_battery,
                        "battery_percent": power.battery_percent,
                    },
                    "identity_ready": identity_ready,
                    "identity_trust_path": identity::trust_path(),
                    "policy_allowed": policy_allowed,
                    "policy_reason": policy_reason(
                        &config,
                        &power,
                        active_model.as_deref(),
                        identity_ready
                    ),
                    "ready_for_jobs": readiness.ready_for_jobs,
                    "readiness_reason": readiness.readiness_reason,
                    "active_model": active_model,
                    "contributed_cluster": config.contributed_cluster,
                    "active_model_compatibility": readiness.model_compatibility,
                    "active_model_compatibility_reason": readiness.model_compatibility_reason,
                });
                if let Err(error) = print_json(&payload) {
                    eprintln!("failed to print json: {error}");
                    std::process::exit(1);
                }
                return;
            }

            print_config_summary(&config, &resolved_config_path());
            println!("routingMode: local-only");
            println!(
                "selectedProvider: {}",
                if provider_count == 1 { "self" } else { "none" }
            );
        }
        Commands::Doctor { json } => {
            let config = current_config_or_default();
            print_doctor_report(&config, json);
        }
        Commands::Logs { json } => {
            print_logs_report(json);
        }
        Commands::Model { command } => {
            let mut config = current_config_or_default();
            match command {
                ModelCommands::List { json } => {
                    let models = match list_models(&config) {
                        Ok(models) => models,
                        Err(error) => {
                            eprintln!("failed to read model cache: {error}");
                            std::process::exit(1);
                        }
                    };
                    print_model_inventory(&config, &models, json);
                }
                ModelCommands::Use { name } => {
                    let name = model_name_for_command(&config, name, true);
                    let backend = resolved_backend(&config);
                    if let Err(error) = ensure_catalog_model_fits_machine(&name, backend, &config) {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = prefetch_mlx_catalog_model(&config, &name) {
                        eprintln!("failed to download MLX model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = prefetch_vllm_catalog_model(&config, &name) {
                        eprintln!("failed to download vLLM model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = use_model(&mut config, &name) {
                        eprintln!("failed to activate model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    print_model_event(
                        "MODEL SWITCHED",
                        &name,
                        "activated and ready for worker launch",
                        Color::Green,
                        &config,
                    );
                }
                ModelCommands::Add { name } => {
                    let name = model_name_for_command(&config, name, false);
                    let backend = resolved_backend(&config);
                    if let Err(error) = ensure_catalog_model_fits_machine(&name, backend, &config) {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = prefetch_mlx_catalog_model(&config, &name) {
                        eprintln!("failed to download MLX model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = prefetch_vllm_catalog_model(&config, &name) {
                        eprintln!("failed to download vLLM model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = add_model(&mut config, &name) {
                        eprintln!("failed to cache model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    print_model_event(
                        "MODEL CACHED",
                        &name,
                        "added to local cache without switching",
                        Color::Cyan,
                        &config,
                    );
                }
                ModelCommands::Import {
                    path,
                    name,
                    backend,
                    vram_mb,
                    activate,
                } => {
                    let backend = backend.unwrap_or_else(detect_backend);
                    let available_vram_mb = vram_mb.or_else(|| {
                        if backend == Backend::Cuda {
                            model_vram_budget_mb(&config, backend)
                        } else {
                            None
                        }
                    });
                    let path = std::path::PathBuf::from(path);
                    let record = match import_model(
                        &mut config,
                        ImportModelOptions {
                            name: name.as_deref(),
                            path: &path,
                            active: activate,
                            backend,
                            available_vram_mb,
                        },
                    ) {
                        Ok(record) => record,
                        Err(error) => {
                            eprintln!("failed to import model `{}`: {error}", path.display());
                            std::process::exit(1);
                        }
                    };
                    if let Err(error) = save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    print_model_event(
                        "MODEL IMPORTED",
                        &record.name,
                        record
                            .compatibility_reason
                            .as_deref()
                            .unwrap_or("local model metadata recorded"),
                        Color::Cyan,
                        &config,
                    );
                }
                ModelCommands::Remove { name, force } => {
                    match remove_model(&mut config, &name, force) {
                        Ok(true) => {
                            if let Err(error) = save_config(&config) {
                                eprintln!("failed to save config: {error}");
                                std::process::exit(1);
                            }
                            print_model_event(
                                "MODEL REMOVED",
                                &name,
                                "cache entry deleted",
                                Color::DarkYellow,
                                &config,
                            );
                        }
                        Ok(false) => {
                            eprintln!("model not found: {name}");
                            std::process::exit(1);
                        }
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(1);
                        }
                    }
                }
                ModelCommands::Prune { yes } => {
                    if !yes {
                        eprintln!("refusing to prune without --yes");
                        std::process::exit(1);
                    }
                    match prune_models(&mut config) {
                        Ok(removed) => {
                            if let Err(error) = save_config(&config) {
                                eprintln!("failed to save config: {error}");
                                std::process::exit(1);
                            }
                            print_model_event(
                                "MODEL PRUNED",
                                &format!("{removed} removed"),
                                "inactive cache entries cleared",
                                Color::DarkYellow,
                                &config,
                            );
                        }
                        Err(error) => {
                            eprintln!("failed to prune models: {error}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Runtime { command } => match command {
            RuntimeCommands::Install { runtime } => {
                let mut config = current_config_or_default();
                match runtime {
                    RuntimeSelection::Mlx => match install_mlx_runtime(&mut config) {
                        Ok(path) => match save_config(&config) {
                            Ok(config_path) => {
                                println!("runtime: {}", runtime.as_str());
                                println!("runtimeReady: yes");
                                println!("runtimePython: {}", path.display());
                                println!("runtimePreference: mlx");
                                println!("runtimeFallback: llama-metal");
                                println!("config: {}", config_path.display());
                            }
                            Err(error) => {
                                eprintln!("failed to save runtime preference: {error}");
                                std::process::exit(1);
                            }
                        },
                        Err(error) => {
                            eprintln!("{error}");
                            eprintln!("runtimeHint: fallback is llama-metal; retry with `opengpu runtime install mlx`");
                            std::process::exit(1);
                        }
                    },
                }
            }
        },
        Commands::Cluster { command } => match command {
            ClusterCommands::Scan { url, json } => run_cluster_scan(url.as_deref(), json),
            ClusterCommands::Use {
                url,
                model,
                max_jobs,
            } => run_cluster_use(&url, model, max_jobs),
            ClusterCommands::Forget => run_cluster_forget(),
        },
        Commands::Update => {
            if let Err(error) = updater::update_installed_binaries() {
                eprintln!("update failed: {error}");
                eprintln!(
                    "updateHint: set OPENGPU_RELEASE_BASE_URL to a trusted release mirror if needed"
                );
                std::process::exit(1);
            }
        }
        Commands::Run {
            prompt,
            model,
            backend,
            max_tokens,
            decompose,
            execution_mode,
            routing,
            timeout,
            interval,
            json,
        } => {
            let config = current_config_or_default();
            let default_model = effective_active_model(&config);
            let requested_model = model.as_deref().or(default_model.as_deref());
            let max_tokens_source = max_tokens_source(max_tokens);
            let max_tokens = effective_max_tokens(&prompt, max_tokens);
            let execution_mode = effective_execution_mode(decompose, execution_mode);
            match run_inference(
                &config,
                &prompt,
                requested_model,
                backend,
                max_tokens,
                max_tokens_source,
                execution_mode,
                routing,
                timeout,
                interval,
            ) {
                Ok(result) => {
                    if json {
                        let output = serde_json::json!({
                            "prompt": prompt,
                            "output": result.output,
                            "node": result.node_label,
                            "assigned_node": result.node_label,
                            "model": result.model_name,
                            "job_id": result.job_id,
                            "status": result.status,
                            "error": result.error,
                            "runtime_metrics": result.runtime_metrics,
                            "routing": result.routing,
                            "local_fallback_code": result.local_fallback_code,
                            "local_fallback_reason": result.local_fallback_reason,
                        });
                        if let Err(error) = print_json(&output) {
                            eprintln!("{error}");
                            std::process::exit(1);
                        }
                    } else {
                        theme::section("Run result");
                        theme::field("assignedNode", &result.node_label);
                        theme::field("routing", &result.routing);
                        if let Some(model_name) = &result.model_name {
                            theme::field("model", model_name);
                        }
                        if let Some(job_id) = &result.job_id {
                            theme::field("jobId", job_id);
                        }
                        if let Some(status) = &result.status {
                            theme::field("status", theme::status(status));
                        }
                        print_runtime_metrics(result.runtime_metrics.as_ref());
                        print_job_plan_progress(&result.job_payload);
                        println!();
                        print_inference_output(&result.output);
                    }
                }
                Err(error) => {
                    theme::error(format!("run failed: {error}"));
                    std::process::exit(1);
                }
            }
        }
        Commands::Jobs { command } => {
            let config = current_config_or_default();
            let result = match command {
                JobsCommands::Submit {
                    prompt,
                    model,
                    backend,
                    max_tokens,
                    decompose,
                    execution_mode,
                    json,
                } => {
                    let default_model = effective_active_model(&config);
                    let requested_model = model.as_deref().or(default_model.as_deref());
                    let max_tokens_source = max_tokens_source(max_tokens);
                    let max_tokens = effective_max_tokens(&prompt, max_tokens);
                    let execution_mode = effective_execution_mode(decompose, execution_mode);
                    submit_job(
                        &config,
                        &prompt,
                        requested_model,
                        backend,
                        max_tokens,
                        max_tokens_source,
                        execution_mode,
                    )
                }
                .and_then(|payload| print_job_response(&payload, json).map(|_| payload)),
                JobsCommands::Status { job_id, json } => get_job(&config, &job_id)
                    .and_then(|payload| print_job_response(&payload, json).map(|_| payload)),
                JobsCommands::Wait {
                    job_id,
                    timeout,
                    interval,
                    json,
                } => wait_for_job(&config, &job_id, timeout, interval)
                    .and_then(|payload| print_job_response(&payload, json).map(|_| payload)),
            };

            if let Err(error) = result {
                theme::error(format!("jobs command failed: {error}"));
                std::process::exit(1);
            }
        }
        Commands::Config { command } => {
            let mut config = current_config_or_default();
            match command {
                ConfigCommands::ModelDir { path } => {
                    let expanded = if path.starts_with('~') {
                        if let Some(home) = dirs::home_dir() {
                            home.join(path.trim_start_matches("~/"))
                                .display()
                                .to_string()
                        } else {
                            path.clone()
                        }
                    } else {
                        path.clone()
                    };
                    config.model_dir = Some(expanded.clone());
                    if let Err(error) = crate::config::save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    println!("modelDir: {expanded}");
                }
                ConfigCommands::ControlPlaneUrl { url } => {
                    let normalized = normalize_control_plane_url(&url).unwrap_or_else(|error| {
                        eprintln!("{error}");
                        std::process::exit(2);
                    });
                    config.control_plane_url = normalized.clone();
                    if let Err(error) = crate::config::save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    println!("controlPlaneUrl: {normalized}");
                }
            }
        }
        Commands::Credits { json, limit, page } => {
            let config = current_config_or_default();
            let node_id = current_node_id(&config);
            let token = auth_token::effective_operator_token(&config);
            match operator_get_json(&config.control_plane_url, "/v1/credits", token.as_deref()) {
                Ok(payload) => {
                    let sync = match sync_local_credit_log(
                        &payload,
                        &node_id,
                        page,
                        limit,
                        now_millis(),
                    ) {
                        Ok(sync) => sync,
                        Err(error) => {
                            eprintln!("failed to sync local credit log: {error}");
                            std::process::exit(1);
                        }
                    };
                    let local_payload = local_credits_payload(&payload, &node_id, &sync);
                    if json {
                        if let Err(error) = print_json(&local_payload) {
                            eprintln!("failed to print json: {error}");
                            std::process::exit(1);
                        }
                        return;
                    }
                    print_local_credits_report(&local_payload);
                }
                Err(error) => {
                    eprintln!("failed to fetch credits: {error}");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        active_graph_node_name, build_job_submission_payload, classify_contributed_cluster,
        cluster, cluster_choice_flag, contribute_detected_cluster, contributed_cluster_from,
        control_plane_endpoint, cuda_doctor_payload, doctor_payload, effective_active_model,
        graph_progress_counts, handles_terminal_key, is_hugging_face_model_id,
        job_degradation_message, job_is_terminal, job_status_path, job_wait_progress_signature,
        local_agent_failure_from_body, local_readiness, logs_payload, may_fallback_to_network,
        network_route_is_unreachable, no_execution_route_error, normalize_control_plane_url,
        parse_worker_output, refresh_contributed_cluster_context, remote_job_output,
        resolve_install_control_plane_url, runtime_metrics_from_output,
        runtime_metrics_from_payload, should_prefetch_vllm_catalog_model,
        should_prompt_model_selection, should_retry_local_offline, should_try_local,
        start_preflight_blockers, sync_config_identity, terminal_line_endings,
        vllm_doctor_payload, Cli, ClusterCommands, Commands, ContributedCluster,
        ContributedClusterCheck, ExecutionMode, JobsCommands, LocalAttemptFailure, PowerState,
        RequestRoutingMode, PUBLIC_CONTROL_PLANE_URL,
    };
    use crate::config::Config;
    use crate::identity::DeviceIdentity;
    use crate::model::ModelRecord;
    use crate::types::Backend;
    use clap::Parser;
    use crossterm::event::KeyEventKind;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn terminal_menus_ignore_windows_key_release_events() {
        assert!(handles_terminal_key(KeyEventKind::Press));
        assert!(handles_terminal_key(KeyEventKind::Repeat));
        assert!(!handles_terminal_key(KeyEventKind::Release));
    }

    fn ac_power() -> PowerState {
        PowerState {
            source: "AC Power".to_string(),
            on_battery: false,
            battery_percent: Some(100),
        }
    }

    fn ready_config() -> Config {
        Config {
            connected: true,
            paused: false,
            contribution_percent: 80,
            active_model: Some("Qwen/Qwen2.5-1.5B-Instruct".to_string()),
            ..Config::default()
        }
    }

    #[test]
    fn exit_alias_maps_to_disconnect() {
        let cli = Cli::try_parse_from(["opengpu", "exit"]).expect("exit alias should parse");
        assert!(matches!(cli.command, Commands::Disconnect));
    }

    #[test]
    fn pause_and_resume_commands_parse() {
        assert!(matches!(
            Cli::try_parse_from(["opengpu", "pause"])
                .expect("pause should parse")
                .command,
            Commands::Pause
        ));
        assert!(matches!(
            Cli::try_parse_from(["opengpu", "resume"])
                .expect("resume should parse")
                .command,
            Commands::Resume
        ));
    }

    #[test]
    fn local_readiness_reports_ready_node() {
        let config = ready_config();
        let readiness = local_readiness(&config, &ac_power(), config.active_model.as_deref(), true);

        assert!(readiness.ready_for_jobs);
        assert_eq!(readiness.readiness_reason, None);
    }

    #[test]
    fn local_readiness_reports_disconnected_node() {
        let mut config = ready_config();
        config.connected = false;

        let readiness = local_readiness(&config, &ac_power(), config.active_model.as_deref(), true);

        assert!(!readiness.ready_for_jobs);
        assert_eq!(
            readiness.readiness_reason.as_deref(),
            Some("node is disconnected")
        );
    }

    #[test]
    fn local_readiness_reports_rejected_active_model() {
        let base =
            std::env::temp_dir().join(format!("opengpu-cli-readiness-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".opengpu")).expect("test model manifest dir");

        let rejected_model = ModelRecord {
            name: "too-big".to_string(),
            active: true,
            cached_at: "test".to_string(),
            model_dir: base.display().to_string(),
            source_path: None,
            file_name: None,
            format: Some("gguf".to_string()),
            quantization: Some("q4".to_string()),
            size_bytes: None,
            estimated_vram_mb: Some(65536),
            compatibility: Some("rejected".to_string()),
            compatibility_reason: Some("requires more VRAM than this node can offer".to_string()),
        };
        fs::write(
            base.join(".opengpu").join("too-big.json"),
            serde_json::to_string(&rejected_model).expect("serialize rejected model"),
        )
        .expect("write rejected model manifest");

        let mut config = ready_config();
        config.active_model = Some("too-big".to_string());
        config.model_dir = Some(base.display().to_string());

        let readiness = local_readiness(&config, &ac_power(), config.active_model.as_deref(), true);
        let _ = fs::remove_dir_all(&base);

        assert!(!readiness.ready_for_jobs);
        assert_eq!(readiness.model_compatibility.as_deref(), Some("rejected"));
        assert_eq!(
            readiness.readiness_reason.as_deref(),
            Some("requires more VRAM than this node can offer")
        );
    }

    #[test]
    fn doctor_command_parses() {
        let cli = Cli::try_parse_from(["opengpu", "doctor"]).expect("doctor should parse");
        assert!(matches!(cli.command, Commands::Doctor { json: false }));
    }

    #[test]
    fn global_theme_flag_parses_without_changing_json_flags() {
        let cli = Cli::try_parse_from(["opengpu", "--theme", "classic", "doctor", "--json"])
            .expect("theme flag should parse");

        assert_eq!(cli.theme, super::theme::ThemeSelection::Classic);
        assert!(matches!(cli.command, Commands::Doctor { json: true }));
    }

    #[test]
    fn existing_config_is_synchronized_with_secure_identity() {
        let mut config = Config {
            device_id: "stale-device".to_string(),
            public_key_fingerprint: Some("stale-fingerprint".to_string()),
            contribution_percent: 80,
            ..Config::default()
        };
        let identity = DeviceIdentity {
            public_key_hex: "01".repeat(32),
            private_key_hex: String::new(),
            fingerprint: "0123456789abcdef".to_string(),
            keychain_label_hex: None,
            encrypted_private_key_hex: "encrypted".to_string(),
            nonce_hex: String::new(),
        };

        sync_config_identity(&mut config, &identity);

        assert_eq!(config.device_id, "node-0123456789abcdef");
        assert_eq!(
            config.public_key_fingerprint.as_deref(),
            Some("0123456789abcdef")
        );
        assert_eq!(config.contribution_percent, 80);
    }

    #[test]
    fn mundusx_theme_and_legacy_reactor_alias_parse() {
        let branded = Cli::try_parse_from(["opengpu", "--theme", "mundusx", "status"])
            .expect("MundusX theme should parse");
        assert_eq!(branded.theme, super::theme::ThemeSelection::Mundusx);

        let legacy = Cli::try_parse_from(["opengpu", "--theme", "reactor", "status"])
            .expect("legacy reactor alias should parse");
        assert_eq!(legacy.theme, super::theme::ThemeSelection::Mundusx);
    }

    #[test]
    fn logs_command_parses() {
        let cli = Cli::try_parse_from(["opengpu", "logs", "--json"]).expect("logs should parse");
        assert!(matches!(cli.command, Commands::Logs { json: true }));
    }

    #[test]
    fn credits_command_parses_local_pagination_flags() {
        let cli = Cli::try_parse_from(["opengpu", "credits", "--limit", "10", "--page", "2"])
            .expect("credits should parse");
        assert!(matches!(
            cli.command,
            Commands::Credits {
                json: false,
                limit: 10,
                page: 2,
            }
        ));
    }

    #[test]
    fn start_debug_flag_parses() {
        let cli = Cli::try_parse_from(["opengpu", "start", "--debug"]).expect("start should parse");
        assert!(matches!(
            cli.command,
            Commands::Start {
                background: false,
                debug: true,
                ..
            }
        ));
    }

    #[test]
    fn start_background_flag_parses() {
        let cli =
            Cli::try_parse_from(["opengpu", "start", "--background"]).expect("start should parse");
        assert!(matches!(
            cli.command,
            Commands::Start {
                background: true,
                debug: false,
                ..
            }
        ));
    }

    #[test]
    fn install_command_parses_public_cap() {
        let cli = Cli::try_parse_from(["opengpu", "install", "--public", "--cap-percent", "30"])
            .expect("install should parse");

        assert!(matches!(
            cli.command,
            Commands::Install {
                public: true,
                private: false,
                control_plane_url: None,
                cap_percent: Some(30),
                ..
            }
        ));
    }

    #[test]
    fn install_command_parses_cluster_contribution_flags() {
        let cli = Cli::try_parse_from([
            "opengpu",
            "install",
            "--public",
            "--contribute-cluster",
            "--cluster-url",
            "http://127.0.0.1:11434",
        ])
        .expect("install should parse cluster flags");

        match cli.command {
            Commands::Install {
                contribute_cluster,
                no_contribute_cluster,
                cluster_url,
                ..
            } => {
                assert!(contribute_cluster);
                assert!(!no_contribute_cluster);
                assert_eq!(cluster_url.as_deref(), Some("http://127.0.0.1:11434"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn install_rejects_contradicting_cluster_flags() {
        let error = Cli::try_parse_from([
            "opengpu",
            "install",
            "--contribute-cluster",
            "--no-contribute-cluster",
        ]);

        assert!(error.is_err());
    }

    #[test]
    fn start_command_parses_cluster_contribution_flags() {
        let cli = Cli::try_parse_from(["opengpu", "start", "--no-contribute-cluster"])
            .expect("start should parse cluster flags");

        match cli.command {
            Commands::Start {
                contribute_cluster,
                no_contribute_cluster,
                ..
            } => {
                assert!(!contribute_cluster);
                assert!(no_contribute_cluster);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn max_jobs_flag_parses_on_every_contribution_path() {
        let install = Cli::try_parse_from([
            "opengpu",
            "install",
            "--public",
            "--contribute-cluster",
            "--max-jobs",
            "8",
        ])
        .expect("install parses --max-jobs");
        match install.command {
            Commands::Install { max_jobs, .. } => assert_eq!(max_jobs, Some(8)),
            other => panic!("unexpected: {other:?}"),
        }

        let start = Cli::try_parse_from(["opengpu", "start", "--max-jobs", "4"])
            .expect("start parses --max-jobs");
        match start.command {
            Commands::Start { max_jobs, .. } => assert_eq!(max_jobs, Some(4)),
            other => panic!("unexpected: {other:?}"),
        }

        let use_cluster = Cli::try_parse_from([
            "opengpu",
            "cluster",
            "use",
            "http://127.0.0.1:8000",
            "--max-jobs",
            "16",
        ])
        .expect("cluster use parses --max-jobs");
        match use_cluster.command {
            Commands::Cluster {
                command: ClusterCommands::Use { max_jobs, .. },
            } => assert_eq!(max_jobs, Some(16)),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn max_jobs_outside_the_supported_range_is_rejected() {
        // Zero would disable the node silently; very large values would let one
        // flag undo every concurrency limit.
        assert!(Cli::try_parse_from(["opengpu", "install", "--max-jobs", "0"]).is_err());
        assert!(Cli::try_parse_from(["opengpu", "install", "--max-jobs", "65"]).is_err());
        assert!(Cli::try_parse_from(["opengpu", "install", "--max-jobs", "1"]).is_ok());
        assert!(Cli::try_parse_from(["opengpu", "install", "--max-jobs", "64"]).is_ok());
    }

    #[test]
    fn the_node_keeps_the_chosen_job_limit_over_runtime_ceilings() {
        let detected = cluster::DetectedCluster {
            kind: cluster::ClusterKind::Vllm,
            base_url: "http://127.0.0.1:8000".to_string(),
            models: vec![cluster::ModelInfo::new("qwen3-coder", None, None)],
            served_context_tokens: Some(131_072),
            memory_utilization: Some(0.85),
            max_concurrency: Some(71),
            max_num_seqs: None,
            kv_cache_tokens: Some(9_435_151),
            supports_tool_calls: true,
        };

        let mut config = Config::default();
        contribute_detected_cluster(&mut config, &detected, Some(8));

        // The runtime says 71; the contributor said 8, and that is a property
        // of the node rather than of the cluster record.
        assert_eq!(config.max_jobs, Some(8));
        assert_eq!(
            config
                .contributed_cluster
                .as_ref()
                .and_then(|cluster| cluster.max_concurrency),
            Some(71)
        );
        assert_eq!(
            config
                .contributed_cluster
                .as_ref()
                .and_then(|cluster| cluster.kv_cache_tokens),
            Some(9_435_151)
        );
    }

    #[test]
    fn cluster_commands_parse() {
        let scan = Cli::try_parse_from(["opengpu", "cluster", "scan", "--json"])
            .expect("cluster scan should parse");
        assert!(matches!(
            scan.command,
            Commands::Cluster {
                command: ClusterCommands::Scan {
                    url: None,
                    json: true
                }
            }
        ));

        let forget =
            Cli::try_parse_from(["opengpu", "cluster", "forget"]).expect("cluster forget parses");
        assert!(matches!(
            forget.command,
            Commands::Cluster {
                command: ClusterCommands::Forget
            }
        ));

        let use_cluster = Cli::try_parse_from([
            "opengpu",
            "cluster",
            "use",
            "http://127.0.0.1:1234",
            "--model",
            "qwen2.5-7b",
        ])
        .expect("cluster use parses");
        match use_cluster.command {
            Commands::Cluster {
                command: ClusterCommands::Use { url, model, .. },
            } => {
                assert_eq!(url, "http://127.0.0.1:1234");
                assert_eq!(model.as_deref(), Some("qwen2.5-7b"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    fn recorded_cluster(model: &str) -> ContributedCluster {
        ContributedCluster {
            kind: "llama.cpp".to_string(),
            base_url: "http://127.0.0.1:8000".to_string(),
            capacity_class: "server".to_string(),
            models: vec![model.to_string()],
            model: Some(model.to_string()),
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: Some(1536),
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        }
    }

    fn probed(models: &[&str]) -> cluster::DetectedCluster {
        cluster::DetectedCluster {
            kind: cluster::ClusterKind::Vllm,
            base_url: "http://127.0.0.1:8000".to_string(),
            models: models
                .iter()
                .map(|name| cluster::ModelInfo::new(*name, None, None))
                .collect(),
            served_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
        }
    }

    #[test]
    fn a_cluster_still_serving_its_model_needs_no_prompt() {
        let recorded = recorded_cluster("UD-IQ2_M");

        assert_eq!(
            classify_contributed_cluster(Some(&recorded), Some(probed(&["UD-IQ2_M"]))),
            ContributedClusterCheck::Unchanged
        );
    }

    #[test]
    fn reverify_refreshes_an_adopted_clusters_runtime_capacity() {
        let mut config = Config::default();
        config.contributed_cluster = Some(recorded_cluster("qwen3-coder"));
        let mut detected = probed(&["qwen3-coder"]);
        detected.max_num_seqs = Some(32);
        detected.max_concurrency = Some(25);
        detected.memory_utilization = Some(0.85);

        refresh_contributed_cluster_context(&mut config, &detected);

        let cluster = config.contributed_cluster.as_ref().expect("cluster");
        assert_eq!(cluster.max_num_seqs, Some(32));
        assert_eq!(cluster.max_concurrency, Some(25));
        assert_eq!(cluster.memory_utilization, Some(0.85));
    }

    #[test]
    fn a_swapped_model_is_detected_so_start_can_ask() {
        // The endpoint answers, but it runs something else now.
        let recorded = recorded_cluster("UD-IQ2_M");
        let probe = probed(&["Qwen/Qwen3-Coder-Next-FP8"]);

        match classify_contributed_cluster(Some(&recorded), Some(probe)) {
            ContributedClusterCheck::ModelChanged { recorded, detected } => {
                assert_eq!(recorded, "UD-IQ2_M");
                assert_eq!(detected.primary_model(), Some("Qwen/Qwen3-Coder-Next-FP8"));
            }
            other => panic!("expected a model change, got {other:?}"),
        }
    }

    #[test]
    fn a_silent_endpoint_is_reported_as_unreachable() {
        let recorded = recorded_cluster("UD-IQ2_M");

        assert_eq!(
            classify_contributed_cluster(Some(&recorded), None),
            ContributedClusterCheck::Unreachable {
                base_url: "http://127.0.0.1:8000".to_string()
            }
        );
    }

    #[test]
    fn a_node_with_no_contributed_cluster_is_left_alone() {
        assert_eq!(
            classify_contributed_cluster(None, None),
            ContributedClusterCheck::NotContributed
        );
    }

    #[test]
    fn a_cluster_serving_several_models_keeps_the_recorded_one() {
        // The advertised model is still there alongside others: no change.
        let recorded = recorded_cluster("hermes3:8b");
        let probe = probed(&["hermes3:70b", "hermes3:8b"]);

        assert_eq!(
            classify_contributed_cluster(Some(&recorded), Some(probe)),
            ContributedClusterCheck::Unchanged
        );
    }

    #[test]
    fn preflight_passes_for_a_ready_cluster_node() {
        let mut config = Config::default();
        config.contribution_percent = 30;
        config.contributed_cluster = Some(ContributedCluster {
            kind: "llama.cpp".to_string(),
            base_url: "http://127.0.0.1:8000".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["UD-IQ2_M".to_string()],
            model: Some("UD-IQ2_M".to_string()),
            model_params: Some(753_864_139_008),
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        });

        assert!(start_preflight_blockers(&config, true, true, Some(true)).is_empty());
    }

    #[test]
    fn preflight_reports_a_cluster_that_stopped_answering() {
        let mut config = Config::default();
        config.contribution_percent = 30;
        config.contributed_cluster = Some(ContributedCluster {
            kind: "llama.cpp".to_string(),
            base_url: "http://127.0.0.1:8000".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["UD-IQ2_M".to_string()],
            model: Some("UD-IQ2_M".to_string()),
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        });

        let silent = start_preflight_blockers(&config, true, true, None);
        assert_eq!(silent.len(), 1);
        assert!(silent[0].contains("is not answering"));

        let wrong_model = start_preflight_blockers(&config, true, true, Some(false));
        assert_eq!(wrong_model.len(), 1);
        assert!(wrong_model[0].contains("is not serving `UD-IQ2_M`"));
    }

    #[test]
    fn preflight_reports_everything_start_would_need() {
        // No identity, no agent binary, no cap, no model and no cluster.
        let blockers = start_preflight_blockers(&Config::default(), false, false, None);

        assert_eq!(blockers.len(), 4);
        assert!(blockers.iter().any(|b| {
            b.contains("device identity") && b.contains("creates it automatically")
        }));
        assert!(blockers.iter().any(|b| b.contains("opengpu-node-agent")));
        assert!(blockers.iter().any(|b| b.contains("contribution cap")));
        assert!(blockers.iter().any(|b| b.contains("no active model")));
    }

    #[test]
    fn preflight_does_not_ask_a_cluster_node_for_a_local_model() {
        let mut config = Config::default();
        config.contribution_percent = 30;
        config.contributed_cluster = Some(ContributedCluster {
            kind: "ollama".to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["hermes3:70b".to_string()],
            model: Some("hermes3:70b".to_string()),
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        });

        let blockers = start_preflight_blockers(&config, true, true, Some(true));

        assert!(!blockers.iter().any(|b| b.contains("active model")));
    }

    #[test]
    fn cluster_choice_flag_maps_both_directions() {
        assert_eq!(cluster_choice_flag(true, false), Some(true));
        assert_eq!(cluster_choice_flag(false, true), Some(false));
        assert_eq!(cluster_choice_flag(false, false), None);
    }

    #[test]
    fn contributed_cluster_supplies_the_effective_active_model() {
        let mut config = Config::default();
        config.contributed_cluster = Some(ContributedCluster {
            kind: "ollama".to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["llama3.1:8b".to_string(), "mistral:7b".to_string()],
            model: None,
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        });

        assert_eq!(
            effective_active_model(&config).as_deref(),
            Some("llama3.1:8b")
        );
    }

    #[test]
    fn explicit_cluster_model_wins_over_the_first_listed_model() {
        let mut config = Config::default();
        config.contributed_cluster = Some(ContributedCluster {
            kind: "lm-studio".to_string(),
            base_url: "http://127.0.0.1:1234".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["a".to_string(), "b".to_string()],
            model: Some("b".to_string()),
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        });

        assert_eq!(effective_active_model(&config).as_deref(), Some("b"));
    }

    #[test]
    fn contributing_a_cluster_skips_the_mundusx_model_selector() {
        let mut config = Config::default();
        config.model_dir = Some(
            std::env::temp_dir()
                .join(format!("opengpu-empty-models-{}", uuid::Uuid::new_v4()))
                .display()
                .to_string(),
        );
        assert!(should_prompt_model_selection(&config));

        config.contributed_cluster = Some(ContributedCluster {
            kind: "vllm".to_string(),
            base_url: "http://127.0.0.1:8000".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["qwen2.5-7b".to_string()],
            model: None,
            model_params: None,
            model_bytes: None,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: None,
        });

        assert!(!should_prompt_model_selection(&config));
    }

    #[test]
    fn detected_cluster_becomes_a_contributed_record() {
        let detected = cluster::DetectedCluster {
            kind: cluster::ClusterKind::Ollama,
            base_url: "http://127.0.0.1:11434".to_string(),
            models: vec![cluster::ModelInfo::new(
                "llama3.1:8b",
                Some(8_000_000_000),
                None,
            )],
            served_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
        };

        let contributed = contributed_cluster_from(&detected, None);

        assert_eq!(contributed.kind, "ollama");
        assert_eq!(contributed.base_url, "http://127.0.0.1:11434");
        assert_eq!(contributed.capacity_class, "server");
        assert_eq!(contributed.model.as_deref(), Some("llama3.1:8b"));
        assert!(contributed.adopted_at.is_some());
    }

    #[test]
    fn contributed_cluster_survives_a_config_round_trip() {
        let mut config = Config::default();
        config.contributed_cluster = Some(ContributedCluster {
            kind: "ollama".to_string(),
            base_url: "http://127.0.0.1:11434".to_string(),
            capacity_class: "server".to_string(),
            models: vec!["llama3.1:8b".to_string()],
            model: Some("llama3.1:8b".to_string()),
            model_params: Some(8_000_000_000),
            model_bytes: None,
            model_capabilities: vec!["tools".to_string()],
            model_context_tokens: Some(131_072),
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: Some("1".to_string()),
        });
        config.cluster_prompt_declined = false;

        let encoded = serde_json::to_string(&config).expect("serialize config");
        let decoded: Config = serde_json::from_str(&encoded).expect("deserialize config");

        assert_eq!(decoded.contributed_cluster, config.contributed_cluster);
    }

    #[test]
    fn configs_written_before_cluster_support_still_load() {
        let legacy = serde_json::json!({
            "version": 1,
            "device_id": "node-legacy",
            "public_key_fingerprint": null,
            "profile_name": null,
            "auth_token": null,
            "connected": false,
            "paused": false,
            "backend_preference": "auto",
            "contribution_percent": 30,
            "control_plane_url": "https://uat.mundusx.ai",
        });

        let decoded: Config =
            serde_json::from_value(legacy).expect("legacy config should still load");

        assert!(decoded.contributed_cluster.is_none());
        assert!(!decoded.cluster_prompt_declined);
    }

    #[test]
    fn cap_command_parses_positional_percent() {
        let cli = Cli::try_parse_from(["opengpu", "cap", "80"]).expect("cap should parse");

        assert!(matches!(
            cli.command,
            Commands::Cap {
                value: Some(80),
                percent: None,
                reset: false,
            }
        ));
    }

    #[test]
    fn cap_command_parses_percent_flag() {
        let cli =
            Cli::try_parse_from(["opengpu", "cap", "--percent", "80"]).expect("cap should parse");

        assert!(matches!(
            cli.command,
            Commands::Cap {
                value: None,
                percent: Some(80),
                reset: false,
            }
        ));
    }

    #[test]
    fn install_public_uses_hosted_control_plane() {
        assert_eq!(
            resolve_install_control_plane_url(true, false, None),
            PUBLIC_CONTROL_PLANE_URL
        );
        assert_eq!(
            resolve_install_control_plane_url(false, false, Some(" ".into())),
            PUBLIC_CONTROL_PLANE_URL
        );
        assert_eq!(
            resolve_install_control_plane_url(false, false, Some("http://127.0.0.1:8787".into())),
            "http://127.0.0.1:8787"
        );
    }

    #[test]
    fn control_plane_url_normalization_accepts_public_blank_and_custom_urls() {
        assert_eq!(
            normalize_control_plane_url(" ").as_deref(),
            Ok(PUBLIC_CONTROL_PLANE_URL)
        );
        assert_eq!(
            normalize_control_plane_url("https://private.example.com/").as_deref(),
            Ok("https://private.example.com")
        );
        assert_eq!(
            normalize_control_plane_url("http://127.0.0.1:8787/").as_deref(),
            Ok("http://127.0.0.1:8787")
        );
    }

    #[test]
    fn control_plane_url_normalization_rejects_missing_scheme() {
        assert!(normalize_control_plane_url("uat.mundusx.ai").is_err());
        assert!(normalize_control_plane_url("ftp://uat.mundusx.ai").is_err());
    }

    #[test]
    fn install_prompts_for_model_only_when_missing_active_model() {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-cli-install-missing-model-{}",
            std::process::id()
        ));
        let mut config = Config::default();
        config.model_dir = Some(temp_dir.display().to_string());
        config.active_model = None;
        assert!(should_prompt_model_selection(&config));

        config.active_model = Some("Qwen/Qwen2.5-1.5B-Instruct".to_string());
        assert!(!should_prompt_model_selection(&config));
    }

    #[test]
    fn mlx_prefetch_accepts_catalog_model_ids() {
        assert!(is_hugging_face_model_id("Qwen/Qwen2.5-0.5B-Instruct"));
        assert!(is_hugging_face_model_id(
            "HuggingFaceTB/SmolLM2-135M-Instruct"
        ));
    }

    #[test]
    fn mlx_prefetch_rejects_local_paths_and_malformed_ids() {
        assert!(!is_hugging_face_model_id("model.gguf"));
        assert!(!is_hugging_face_model_id("/tmp/model"));
        assert!(!is_hugging_face_model_id("owner/../model"));
        assert!(!is_hugging_face_model_id("owner/model/extra"));
    }

    #[test]
    fn vllm_prefetch_accepts_only_vllm_catalog_models() {
        let mut config = Config::default();
        config.backend_preference = Backend::Vllm;
        assert!(should_prefetch_vllm_catalog_model(
            &config,
            "Qwen/Qwen3-Coder-30B-A3B-Instruct"
        ));
        assert!(!should_prefetch_vllm_catalog_model(
            &config,
            "example/not-in-catalog"
        ));

        config.backend_preference = Backend::M;
        assert!(!should_prefetch_vllm_catalog_model(
            &config,
            "Qwen/Qwen3-Coder-30B-A3B-Instruct"
        ));
    }

    #[test]
    fn terminal_output_returns_to_column_zero_in_raw_mode() {
        let mut previous_was_carriage_return = false;
        assert_eq!(
            terminal_line_endings(b"first\nsecond\r\n", &mut previous_was_carriage_return),
            b"first\r\nsecond\r\n"
        );
    }

    #[test]
    fn terminal_output_handles_line_endings_split_across_reads() {
        let mut previous_was_carriage_return = false;
        assert_eq!(
            terminal_line_endings(b"first\r", &mut previous_was_carriage_return),
            b"first\r"
        );
        assert_eq!(
            terminal_line_endings(b"\nsecond\n", &mut previous_was_carriage_return),
            b"\nsecond\r\n"
        );
    }

    #[test]
    fn control_plane_endpoint_accepts_https_and_local_http() {
        assert_eq!(
            control_plane_endpoint("https://uat.mundusx.ai", "/v1/jobs").as_deref(),
            Ok("https://uat.mundusx.ai/v1/jobs")
        );
        assert_eq!(
            control_plane_endpoint("http://127.0.0.1:8787/", "/v1/jobs/job_123").as_deref(),
            Ok("http://127.0.0.1:8787/v1/jobs/job_123")
        );
    }

    #[test]
    fn control_plane_endpoint_rejects_invalid_urls_and_paths() {
        assert!(control_plane_endpoint("uat.mundusx.ai", "/v1/jobs").is_err());
        assert!(control_plane_endpoint("ftp://uat.mundusx.ai", "/v1/jobs").is_err());
        assert!(control_plane_endpoint("https://uat.mundusx.ai", "v1/jobs").is_err());
    }

    #[test]
    fn jobs_submit_command_parses_async_options() {
        let cli = Cli::try_parse_from([
            "opengpu",
            "jobs",
            "submit",
            "--prompt",
            "hello",
            "--model",
            "smol",
            "--backend",
            "cuda",
            "--max-tokens",
            "64",
            "--json",
        ])
        .expect("jobs submit should parse");

        match cli.command {
            Commands::Jobs {
                command:
                    JobsCommands::Submit {
                        prompt,
                        model,
                        backend,
                        max_tokens,
                        decompose,
                        execution_mode,
                        json,
                    },
            } => {
                assert_eq!(prompt, "hello");
                assert_eq!(model.as_deref(), Some("smol"));
                assert_eq!(backend, Backend::Cuda);
                assert_eq!(max_tokens, Some(64));
                assert!(!decompose);
                assert_eq!(execution_mode, None);
                assert!(json);
            }
            _ => panic!("expected jobs submit command"),
        }
    }

    #[test]
    fn model_import_command_parses_compatibility_options() {
        let cli = Cli::try_parse_from([
            "opengpu",
            "model",
            "import",
            "./models/local.gguf",
            "--name",
            "local",
            "--backend",
            "cuda",
            "--vram-mb",
            "4096",
            "--activate",
        ])
        .expect("model import should parse");

        match cli.command {
            Commands::Model {
                command:
                    super::ModelCommands::Import {
                        path,
                        name,
                        backend,
                        vram_mb,
                        activate,
                    },
            } => {
                assert_eq!(path, "./models/local.gguf");
                assert_eq!(name.as_deref(), Some("local"));
                assert_eq!(backend, Some(Backend::Cuda));
                assert_eq!(vram_mb, Some(4096));
                assert!(activate);
            }
            _ => panic!("expected model import command"),
        }
    }

    #[test]
    fn model_use_and_add_allow_picker_without_name() {
        let use_cli = Cli::try_parse_from(["opengpu", "model", "use"])
            .expect("model use should allow picker");
        assert!(matches!(
            use_cli.command,
            Commands::Model {
                command: super::ModelCommands::Use { name: None }
            }
        ));

        let add_cli = Cli::try_parse_from(["opengpu", "model", "add"])
            .expect("model add should allow picker");
        assert!(matches!(
            add_cli.command,
            Commands::Model {
                command: super::ModelCommands::Add { name: None }
            }
        ));
    }

    #[test]
    fn model_use_and_add_keep_direct_name_input() {
        let use_cli =
            Cli::try_parse_from(["opengpu", "model", "use", "Qwen/Qwen2.5-0.5B-Instruct"])
                .expect("model use direct name should parse");
        match use_cli.command {
            Commands::Model {
                command: super::ModelCommands::Use { name },
            } => assert_eq!(name.as_deref(), Some("Qwen/Qwen2.5-0.5B-Instruct")),
            _ => panic!("expected model use command"),
        }

        let add_cli =
            Cli::try_parse_from(["opengpu", "model", "add", "Qwen/Qwen2.5-0.5B-Instruct"])
                .expect("model add direct name should parse");
        match add_cli.command {
            Commands::Model {
                command: super::ModelCommands::Add { name },
            } => assert_eq!(name.as_deref(), Some("Qwen/Qwen2.5-0.5B-Instruct")),
            _ => panic!("expected model add command"),
        }
    }

    #[test]
    fn jobs_status_and_wait_commands_parse() {
        let status = Cli::try_parse_from(["opengpu", "jobs", "status", "job_123", "--json"])
            .expect("jobs status should parse");
        assert!(matches!(
            status.command,
            Commands::Jobs {
                command: JobsCommands::Status { json: true, .. }
            }
        ));

        let wait = Cli::try_parse_from([
            "opengpu",
            "jobs",
            "wait",
            "job-123",
            "--timeout",
            "30",
            "--interval",
            "5",
        ])
        .expect("jobs wait should parse");
        assert!(matches!(
            wait.command,
            Commands::Jobs {
                command: JobsCommands::Wait {
                    timeout: 30,
                    interval: 5,
                    ..
                }
            }
        ));
    }

    #[test]
    fn run_command_parses_remote_wait_options() {
        let cli = Cli::try_parse_from([
            "opengpu",
            "run",
            "--prompt",
            "hello",
            "--timeout",
            "45",
            "--interval",
            "3",
            "--decompose",
            "--json",
        ])
        .expect("run should parse");

        match cli.command {
            Commands::Run {
                prompt,
                timeout,
                interval,
                decompose,
                json,
                ..
            } => {
                assert_eq!(prompt, "hello");
                assert_eq!(timeout, 45);
                assert_eq!(interval, 3);
                assert!(decompose);
                assert!(json);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn run_command_defaults_to_local_first_and_accepts_routing_override() {
        let default_cli =
            Cli::try_parse_from(["opengpu", "run", "--prompt", "hello"]).expect("run should parse");
        assert!(matches!(
            default_cli.command,
            Commands::Run {
                routing: RequestRoutingMode::LocalFirst,
                ..
            }
        ));

        let local_only = Cli::try_parse_from([
            "opengpu",
            "run",
            "--prompt",
            "hello",
            "--routing",
            "local-only",
        ])
        .expect("local-only should parse");
        assert!(matches!(
            local_only.command,
            Commands::Run {
                routing: RequestRoutingMode::LocalOnly,
                ..
            }
        ));

        let network_only = Cli::try_parse_from([
            "opengpu",
            "run",
            "--prompt",
            "hello",
            "--routing",
            "network-only",
        ])
        .expect("network-only should parse");
        assert!(matches!(
            network_only.command,
            Commands::Run {
                routing: RequestRoutingMode::NetworkOnly,
                ..
            }
        ));
    }

    #[test]
    fn routing_policy_tries_only_permitted_execution_paths() {
        assert!(should_try_local(
            RequestRoutingMode::LocalFirst,
            ExecutionMode::Single
        ));
        assert!(should_try_local(
            RequestRoutingMode::LocalOnly,
            ExecutionMode::Single
        ));
        assert!(!should_try_local(
            RequestRoutingMode::NetworkOnly,
            ExecutionMode::Single
        ));
        assert!(!should_try_local(
            RequestRoutingMode::LocalFirst,
            ExecutionMode::Decompose
        ));

        let routable = LocalAttemptFailure::new("LOCAL_CAPACITY_UNAVAILABLE", "busy", true);
        assert!(may_fallback_to_network(
            RequestRoutingMode::LocalFirst,
            &routable
        ));
        assert!(!may_fallback_to_network(
            RequestRoutingMode::LocalOnly,
            &routable
        ));
    }

    #[test]
    fn local_agent_failures_preserve_machine_readable_routing_codes() {
        let routable = local_agent_failure_from_body(
            409,
            r#"{"code":"LOCAL_MODEL_UNSUITABLE","error":"model too small","fallback":"network"}"#,
        );
        assert_eq!(routable.code, "LOCAL_MODEL_UNSUITABLE");
        assert_eq!(routable.message, "model too small");
        assert!(routable.network_fallback);

        let invalid = local_agent_failure_from_body(
            400,
            r#"{"code":"INVALID_LOCAL_REQUEST","error":"prompt is required","fallback":"none"}"#,
        );
        assert!(!invalid.network_fallback);

        let explicitly_terminal = local_agent_failure_from_body(
            409,
            r#"{"code":"LOCAL_POLICY_BLOCKED","error":"blocked","fallback":"none"}"#,
        );
        assert!(!explicitly_terminal.network_fallback);
    }

    #[test]
    fn terminal_route_error_reports_both_exhausted_paths() {
        let local = LocalAttemptFailure::new("LOCAL_RUNTIME_FAILURE", "worker exited", true);
        let error = no_execution_route_error(&local, "control plane unreachable");
        assert!(error.contains("LOCAL_RUNTIME_FAILURE"));
        assert!(error.contains("worker exited"));
        assert!(error.contains("control plane unreachable"));
    }

    #[test]
    fn offline_retry_is_bounded_to_unsuitable_local_models_and_transport_failure() {
        let unsuitable =
            LocalAttemptFailure::new("LOCAL_MODEL_UNSUITABLE", "model too small", true);
        assert!(network_route_is_unreachable(
            "network routing failed: request failed: connection refused"
        ));
        assert!(network_route_is_unreachable(
            "remote job job-1 did not complete: request failed: timed out"
        ));
        assert!(!network_route_is_unreachable(
            "network routing failed: HTTP 500: request failed: provider rejected payload"
        ));
        assert!(should_retry_local_offline(
            RequestRoutingMode::LocalFirst,
            &unsuitable,
            "network routing failed: request failed: connection refused"
        ));
        assert!(!should_retry_local_offline(
            RequestRoutingMode::LocalOnly,
            &unsuitable,
            "network routing failed: request failed: connection refused"
        ));
        assert!(!should_retry_local_offline(
            RequestRoutingMode::LocalFirst,
            &unsuitable,
            "remote job failed: model unavailable"
        ));

        let runtime_failure =
            LocalAttemptFailure::new("LOCAL_RUNTIME_FAILURE", "worker exited", true);
        assert!(!should_retry_local_offline(
            RequestRoutingMode::LocalFirst,
            &runtime_failure,
            "network routing failed: request failed: connection refused"
        ));
    }

    #[test]
    fn omitted_request_model_uses_active_model() {
        let mut config = Config::default();
        config.active_model = Some("Qwen/Qwen2.5-0.5B-Instruct".to_string());

        let default_model = super::active_model_name(&config);
        let requested_model = None.or(default_model.as_deref());

        assert_eq!(requested_model, Some("Qwen/Qwen2.5-0.5B-Instruct"));
    }

    #[test]
    fn job_submission_payload_matches_control_plane_contract() {
        let (_, payload) = build_job_submission_payload(
            "hello",
            Some("smol"),
            Backend::Cuda,
            64,
            "explicit",
            ExecutionMode::Auto,
        );

        assert!(payload["request_id"]
            .as_str()
            .unwrap_or("")
            .starts_with("req-"));
        assert_eq!(payload["prompt"].as_str(), Some("hello"));
        assert_eq!(payload["model"].as_str(), Some("smol"));
        assert_eq!(payload["preferred_backend"].as_str(), Some("cuda"));
        assert_eq!(payload["execution_mode"].as_str(), Some("auto"));
        assert_eq!(payload["max_tokens"].as_u64(), Some(64));
        assert_eq!(payload["max_tokens_source"].as_str(), Some("explicit"));
    }

    #[test]
    fn max_tokens_source_marks_omitted_values_as_auto() {
        assert_eq!(super::max_tokens_source(None), "auto");
        assert_eq!(super::max_tokens_source(Some(64)), "explicit");
    }

    #[test]
    fn run_defaults_short_math_prompts_to_small_generation_budget() {
        assert_eq!(
            super::effective_max_tokens("The answer to 500+31 is", None),
            4
        );
        assert_eq!(super::effective_max_tokens("500+31=", None), 4);
    }

    #[test]
    fn run_defaults_answer_only_prompts_to_short_generation_budget() {
        assert_eq!(
            super::effective_max_tokens("Answer only with the number: 421+31=", None),
            4
        );
        assert_eq!(
            super::effective_max_tokens("Answer only with one word", None),
            16
        );
    }

    #[test]
    fn run_defaults_normal_prompts_to_concise_generation_budget() {
        assert_eq!(
            super::effective_max_tokens("Explain why local inference can be slow", None),
            512
        );
    }

    #[test]
    fn run_defaults_long_form_prompts_to_larger_generation_budget() {
        assert_eq!(
            super::effective_max_tokens(
                "Give me a detailed history of Microsoft from its origins to today.",
                None
            ),
            1536
        );
        assert_eq!(
            super::effective_max_tokens("Write a comprehensive report about GPU markets", None),
            1536
        );
    }

    #[test]
    fn run_defaults_complete_code_prompts_to_full_generation_budget() {
        assert_eq!(
            super::effective_max_tokens(
                "Give me a complete turbo c program to handle enrollment of students save in binary file",
                None
            ),
            2048
        );
    }

    #[test]
    fn explicit_max_tokens_override_auto_budget() {
        assert_eq!(
            super::effective_max_tokens("The answer to 500+31 is", Some(512)),
            512
        );
    }

    #[test]
    fn job_status_path_rejects_unsafe_ids() {
        assert_eq!(
            job_status_path("job_123").as_deref(),
            Ok("/v1/jobs/job_123")
        );
        assert!(job_status_path("../jobs").is_err());
        assert!(job_status_path("job 123").is_err());
        assert!(job_status_path("").is_err());
    }

    #[test]
    fn terminal_job_states_are_detected() {
        assert!(job_is_terminal(&serde_json::json!({"status": "completed"})));
        assert!(job_is_terminal(&serde_json::json!({"status": "failed"})));
        assert!(!job_is_terminal(&serde_json::json!({"status": "queued"})));
        assert!(!job_is_terminal(&serde_json::json!({"state": "assigned"})));
    }

    #[test]
    fn graph_wait_progress_signature_tracks_chunk_progress() {
        let payload = serde_json::json!({
            "status": "queued",
            "job": {
                "status": "queued",
                "graph_execution_enabled": true,
                "active_graph_node_id": "job.early_development",
                "graph": {
                    "nodes": [
                        {"id": "job.origins", "name": "Origins", "status": "completed"},
                        {"id": "job.early_development", "name": "Early development", "status": "running"},
                        {"id": "job.final", "name": "Final synthesis", "status": "waiting"}
                    ]
                }
            }
        });

        assert_eq!(
            active_graph_node_name(&payload).as_deref(),
            Some("Early development")
        );
        assert_eq!(
            job_wait_progress_signature(&payload).as_deref(),
            Some("queued|1|1|3|processing|Early development|Early development/worker=pending")
        );
    }

    #[test]
    fn job_status_reads_structured_degradation_message() {
        let payload = serde_json::json!({
            "status": "queued",
            "job": {
                "status": "queued",
                "degradation": {
                    "code": "NO_CREDIBLE_REDUCER",
                    "message": "Expert work is preserved. Waiting for a qualified reducer before continuing."
                }
            }
        });

        assert_eq!(
            job_degradation_message(&payload),
            Some("Expert work is preserved. Waiting for a qualified reducer before continuing.")
        );
    }

    #[test]
    fn graph_progress_labels_processing_and_merging_nodes() {
        let processing = serde_json::json!({
            "id": "job.origins",
            "name": "Origins and founders",
            "status": "running",
            "responsibility": "research"
        });
        let merging = serde_json::json!({
            "id": "job.final",
            "name": "Final synthesis",
            "status": "running",
            "responsibility": "merge"
        });
        let completed = serde_json::json!({
            "id": "job.origins",
            "name": "Origins and founders",
            "status": "completed"
        });

        assert_eq!(super::graph_node_display_status(&processing), "processing");
        assert_eq!(super::graph_node_display_status(&merging), "merging");
        assert_eq!(super::graph_node_display_status(&completed), "done");
    }

    #[test]
    fn remote_job_output_accepts_final_output_fallbacks() {
        assert_eq!(
            remote_job_output(&serde_json::json!({"status": "completed", "output": "hello"})),
            Some("hello".to_string())
        );
        assert_eq!(
            remote_job_output(&serde_json::json!({"status": "completed", "final_output": "done"})),
            Some("done".to_string())
        );
        assert_eq!(
            remote_job_output(
                &serde_json::json!({"status": "completed", "result": {"text": "ok"}})
            ),
            Some(r#"{"text":"ok"}"#.to_string())
        );
        assert_eq!(
            remote_job_output(
                &serde_json::json!({"status": "completed", "job": {"output": "nested"}})
            ),
            Some("nested".to_string())
        );
        assert_eq!(
            remote_job_output(
                &serde_json::json!({"status": "completed", "job": {"graph": {"final_output": "graph"}}})
            ),
            Some("graph".to_string())
        );
        assert_eq!(
            remote_job_output(&serde_json::json!({"status": "completed", "output": null})),
            None
        );
    }

    #[test]
    fn runtime_metrics_parse_from_worker_output() {
        let output = r#"llama.cpp mode=cuda; runtime_metrics={"total_duration_ms":250.0,"prompt_eval_count":10,"prompt_eval_rate":200.0,"eval_count":8,"eval_rate":50.0}; response=hello"#;

        let metrics = runtime_metrics_from_output(output).expect("runtime metrics");

        assert_eq!(metrics.total_duration_ms, Some(250.0));
        assert_eq!(metrics.prompt_eval_count, Some(10));
        assert_eq!(metrics.prompt_eval_rate, Some(200.0));
        assert_eq!(metrics.eval_count, Some(8));
        assert_eq!(metrics.eval_rate, Some(50.0));
    }

    #[test]
    fn runtime_metrics_aggregate_from_graph_nodes() {
        let payload = serde_json::json!({
            "job": {
                "graph": {
                    "nodes": [
                        {"runtime_metrics": {"total_duration_ms": 100.0, "eval_count": 4, "eval_rate": 40.0}},
                        {"output": "llama.cpp runtime_metrics={\"total_duration_ms\":200.0,\"eval_count\":6,\"eval_rate\":60.0}; response=ok"}
                    ]
                }
            }
        });

        let metrics = runtime_metrics_from_payload(&payload).expect("runtime metrics");

        assert_eq!(metrics.total_duration_ms, Some(300.0));
        assert_eq!(metrics.eval_count, Some(10));
        assert_eq!(metrics.eval_rate, Some(50.0));
    }

    #[test]
    fn llama_output_metadata_is_split_into_display_fields() {
        let output = r#"llama.cpp mode=cuda; model=Qwen; path=C:\model.gguf; max_tokens=16; runtime_metrics={"eval_count":4}; response=hello world"#;

        let (fields, response) = parse_worker_output(output).expect("llama output");

        assert_eq!(
            fields,
            vec![
                ("mode".to_string(), "cuda".to_string()),
                ("model".to_string(), "Qwen".to_string()),
                ("path".to_string(), r#"C:\model.gguf"#.to_string()),
                ("maxTokens".to_string(), "16".to_string()),
            ]
        );
        assert_eq!(response, "hello world");
    }

    #[test]
    fn mlx_output_metadata_is_split_into_display_fields() {
        let output = r#"mlx-lm mode=mlx; model=mlx-community/Qwen2.5-1.5B-Instruct; max_tokens=32; response=hello from mlx"#;

        let (fields, response) = parse_worker_output(output).expect("mlx output");

        assert_eq!(
            fields,
            vec![
                ("mode".to_string(), "mlx".to_string()),
                (
                    "model".to_string(),
                    "mlx-community/Qwen2.5-1.5B-Instruct".to_string()
                ),
                ("maxTokens".to_string(), "32".to_string()),
            ]
        );
        assert_eq!(response, "hello from mlx");
    }

    #[test]
    fn timeout_detail_reports_assigned_node_and_running_chunk() {
        let payload = serde_json::json!({
            "job": {
                "status": "assigned",
                "assigned_node_id": "node-1",
                "active_graph_node_id": "job.origins",
                "graph": {
                    "nodes": [
                        {"id": "job.origins", "name": "Origins and founders", "status": "running", "assigned_node_id": "node-1"},
                        {"id": "job.final_merge", "name": "Final synthesis", "status": "waiting"}
                    ]
                }
            }
        });

        assert_eq!(
            super::timeout_job_detail(&payload),
            " (assignedNode=node-1, activeChunk=job.origins, runningChunks=Origins and founders@node-1/worker=pending)"
        );
    }

    #[test]
    fn running_graph_chunk_summaries_include_all_nodes_and_workers() {
        let payload = serde_json::json!({
            "job": {
                "graph": {
                    "nodes": [
                        {
                            "id": "job.origins",
                            "name": "Origins and founders",
                            "status": "running",
                            "assigned_node_id": "node-1",
                            "worker_id": "worker-a"
                        },
                        {
                            "id": "job.expansion",
                            "name": "Expansion",
                            "status": "running",
                            "assigned_node_id": "node-2"
                        },
                        {"id": "job.final_merge", "name": "Final synthesis", "status": "waiting"}
                    ]
                }
            }
        });

        assert_eq!(
            super::running_graph_chunk_summaries(&payload),
            vec![
                "Origins and founders@node-1/worker=worker-a".to_string(),
                "Expansion@node-2/worker=pending".to_string()
            ]
        );
    }

    #[test]
    fn graph_progress_counts_completed_and_running_nodes() {
        let payload = serde_json::json!({
            "graph": {
                "nodes": [
                    {"name": "Origins", "status": "completed"},
                    {"name": "Modern era", "status": "running"},
                    {"name": "Final synthesis", "status": "waiting"}
                ]
            }
        });

        assert_eq!(graph_progress_counts(&payload), Some((1, 1, 3)));
    }

    #[test]
    fn waiting_graph_nodes_use_dependency_wording_not_blocked_wording() {
        let dependencies = vec!["job.origins", "job.early_development"];

        assert_eq!(
            super::graph_node_dependency_suffix("waiting", &dependencies),
            " waiting for job.origins, job.early_development"
        );
        assert_eq!(
            super::graph_node_dependency_suffix("failed", &dependencies),
            " blocked by job.origins, job.early_development"
        );
        assert_eq!(super::graph_node_dependency_suffix("waiting", &[]), "");
    }

    #[test]
    fn default_contribution_percent_matches_backend_risk() {
        assert_eq!(super::default_contribution_percent(Backend::Cuda), 30);
        assert_eq!(super::default_contribution_percent(Backend::Vulkan), 30);
        assert_eq!(super::default_contribution_percent(Backend::Vllm), 30);
        assert_eq!(super::default_contribution_percent(Backend::M), 30);
        assert_eq!(super::default_contribution_percent(Backend::Auto), 20);
    }

    #[test]
    fn contribution_percent_rejects_dedicated_machine_caps() {
        assert!(super::normalize_contribution_percent(0).is_err());
        assert_eq!(super::normalize_contribution_percent(1), Ok(1));
        assert_eq!(super::normalize_contribution_percent(20), Ok(20));
        assert_eq!(super::normalize_contribution_percent(30), Ok(30));
        assert_eq!(super::normalize_contribution_percent(50), Ok(50));
        assert_eq!(super::normalize_contribution_percent(65), Ok(65));
        assert_eq!(super::normalize_contribution_percent(75), Ok(75));
        assert_eq!(super::normalize_contribution_percent(80), Ok(80));
        assert!(super::normalize_contribution_percent(81).is_err());
    }

    #[test]
    fn install_profile_routes_machine_families() {
        assert_eq!(
            super::install_profile_for("macos", "aarch64", Backend::M),
            "macos-aarch64-apple-silicon"
        );
        assert_eq!(
            super::install_profile_for("windows", "x86_64", Backend::Cuda),
            "windows-x86_64-cuda"
        );
        assert_eq!(
            super::install_profile_for("windows", "x86_64", Backend::Vulkan),
            "windows-x86_64-vulkan"
        );
        assert_eq!(
            super::install_profile_for("linux", "x86_64", Backend::Cuda),
            "linux-x86_64-cuda"
        );
        assert_eq!(
            super::install_profile_for("linux", "x86_64", Backend::Vllm),
            "linux-x86_64-vllm"
        );
        assert_eq!(
            super::install_profile_for("windows", "x86_64", Backend::Auto),
            "windows-x86_64-generic"
        );
    }

    #[test]
    fn installed_vllm_runtime_is_preferred_over_linux_cuda() {
        assert_eq!(
            super::preferred_installed_backend("linux", Backend::Cuda, true),
            Backend::Vllm
        );
        assert_eq!(
            super::preferred_installed_backend("linux", Backend::Cuda, false),
            Backend::Cuda
        );
        assert_eq!(
            super::preferred_installed_backend("windows", Backend::Cuda, true),
            Backend::Cuda
        );
    }

    #[test]
    fn local_model_resolution_uses_imported_source_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-cli-imported-model-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let manifest_dir = temp_dir.join(".opengpu");
        std::fs::create_dir_all(&manifest_dir).expect("manifest dir");
        let source_path = temp_dir.join("external-q4_k_m.gguf");
        std::fs::write(&source_path, b"model").expect("model file");
        std::fs::write(
            manifest_dir.join("external.json"),
            serde_json::json!({
                "name": "external",
                "active": true,
                "cached_at": "1",
                "model_dir": temp_dir,
                "source_path": source_path,
            })
            .to_string(),
        )
        .expect("manifest");

        let resolved =
            super::resolve_local_model_path(&temp_dir, Some("external")).expect("resolve model");

        assert_eq!(resolved, source_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn doctor_payload_reports_expected_paths() {
        let _guard = env_lock().lock().expect("env lock");
        let temp = std::env::temp_dir().join(format!("opengpu-cli-doctor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("temp dir");
        std::env::set_var("OPENGPU_HOME", &temp);

        let payload = doctor_payload(&crate::config::Config::default());

        assert_eq!(payload["config_dir"].as_str(), temp.to_str());
        assert!(payload["resolved_config_parent_writable"]
            .as_bool()
            .unwrap_or(false));
        assert!(payload["model_dir"]
            .as_str()
            .unwrap_or("")
            .starts_with(temp.to_str().unwrap_or("")));
        assert!(Path::new(payload["identity_path"].as_str().unwrap_or("")).starts_with(&temp));

        std::env::remove_var("OPENGPU_HOME");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn cuda_doctor_reports_windows_low_vram_readiness() {
        let payload = cuda_doctor_payload(
            "windows",
            Backend::Cuda,
            Ok("NVIDIA GeForce GTX 1050 Ti\n".to_string()),
            Ok("4096\n".to_string()),
        );

        assert_eq!(payload["selected_backend"].as_str(), Some("cuda"));
        assert_eq!(
            payload["cuda_device_name"].as_str(),
            Some("NVIDIA GeForce GTX 1050 Ti")
        );
        assert_eq!(payload["cuda_vram_mb"].as_u64(), Some(4096));
        assert!(payload["cuda_low_vram_profile"].as_bool().unwrap_or(false));
        assert_eq!(
            payload["runtime_readiness"].as_str(),
            Some("cuda-prerequisites-detected")
        );
        assert!(payload["lm_studio_without_cuda_toolkit_supported"]
            .as_bool()
            .unwrap_or(false));
    }

    #[test]
    fn cuda_doctor_blocks_cuda_when_nvidia_smi_is_missing() {
        let payload = cuda_doctor_payload(
            "windows",
            Backend::Cuda,
            Err("nvidia-smi unavailable: not found".to_string()),
            Err("nvidia-smi unavailable: not found".to_string()),
        );

        assert!(!payload["nvidia_smi_available"].as_bool().unwrap_or(true));
        assert!(!payload["cuda_device_available"].as_bool().unwrap_or(true));
        assert_eq!(
            payload["runtime_readiness"].as_str(),
            Some("blocked-nvidia-smi-unavailable")
        );
        assert!(payload["notes"].as_array().unwrap().iter().any(|note| note
            .as_str()
            .unwrap_or("")
            .contains("install or repair the NVIDIA driver")));
    }

    #[test]
    fn vllm_doctor_blocks_vllm_on_windows_without_touching_cuda_path() {
        let payload = vllm_doctor_payload("windows", Backend::Vllm, true, true, true, true);

        assert_eq!(payload["selected_backend"].as_str(), Some("vllm"));
        assert!(!payload["supported_os"].as_bool().unwrap_or(true));
        assert_eq!(
            payload["runtime_readiness"].as_str(),
            Some("unsupported-on-this-os")
        );
        assert!(payload["notes"].as_array().unwrap().iter().any(|note| note
            .as_str()
            .unwrap_or("")
            .contains("Windows keeps using the llama.cpp runtime path")));
    }

    #[test]
    fn vllm_doctor_reports_linux_dependency_readiness() {
        let payload = vllm_doctor_payload("linux", Backend::Vllm, true, true, true, true);

        assert!(payload["supported_os"].as_bool().unwrap_or(false));
        assert_eq!(
            payload["runtime_readiness"].as_str(),
            Some("vllm-runtime-ready")
        );
    }

    #[test]
    fn logs_payload_points_at_local_agent_files() {
        let _guard = env_lock().lock().expect("env lock");
        let temp = std::env::temp_dir().join(format!("opengpu-cli-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("temp dir");
        std::env::set_var("OPENGPU_HOME", &temp);

        let payload = logs_payload();

        assert_eq!(
            payload["agent_state_path"].as_str(),
            temp.join("agent-state.json").to_str()
        );
        assert_eq!(
            payload["heartbeat_log_path"].as_str(),
            temp.join("heartbeat.jsonl").to_str()
        );

        std::env::remove_var("OPENGPU_HOME");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn local_credit_totals_filter_to_current_node() {
        let payload = serde_json::json!({
            "by_node": {
                "node-current": 2.5,
                "node-other": 99.0
            },
            "ledger": [
                {"id": "earn-1", "device_id": "node-current", "amount": 3.0, "currency": "credits", "created_at": "1"},
                {"id": "use-1", "device_id": "node-current", "amount": -0.5, "currency": "credits", "created_at": "2"},
                {"id": "earn-2", "device_id": "node-other", "amount": 99.0, "currency": "credits", "created_at": "3"}
            ]
        });

        let totals = super::local_credit_totals(&payload, "node-current");

        assert_eq!(totals.node_id, "node-current");
        assert_eq!(totals.remaining_credits, 2.5);
        assert_eq!(totals.earned_credits, 3.0);
        assert_eq!(totals.used_credits, 0.5);
        assert_eq!(totals.ledger_entries, 2);
    }

    #[test]
    fn credit_log_sync_rotates_after_thirty_minutes_and_skips_duplicates() {
        let _guard = env_lock().lock().expect("env lock");
        let temp = std::env::temp_dir().join(format!(
            "opengpu-cli-credits-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).expect("temp dir");
        std::env::set_var("OPENGPU_HOME", &temp);

        let first_payload = serde_json::json!({
            "ledger": [
                {"id": "scope", "device_id": "node-current", "job_id": "job-1", "amount": 1.0, "currency": "credits", "created_at": "1"},
                {"id": "backend", "device_id": "node-current", "job_id": "job-1", "amount": 2.0, "currency": "credits", "created_at": "2"},
                {"id": "other", "device_id": "node-other", "job_id": "job-1", "amount": 3.0, "currency": "credits", "created_at": "3"}
            ]
        });

        let first = super::sync_local_credit_log(&first_payload, "node-current", 0, 1, 1_000)
            .expect("first sync");
        assert_eq!(first.appended_entries, 2);
        assert_eq!(first.current_entries.len(), 1);
        assert_eq!(
            first.current_entries[0]["ledger"]["id"].as_str(),
            Some("backend")
        );

        let duplicate = super::sync_local_credit_log(&first_payload, "node-current", 0, 25, 1_500)
            .expect("duplicate sync");
        assert_eq!(duplicate.appended_entries, 0);
        assert_eq!(super::credit_log_files().len(), 1);

        let rotated_payload = serde_json::json!({
            "ledger": [
                {"id": "scope", "device_id": "node-current", "job_id": "job-1", "amount": 1.0, "currency": "credits", "created_at": "1"},
                {"id": "backend", "device_id": "node-current", "job_id": "job-1", "amount": 2.0, "currency": "credits", "created_at": "2"},
                {"id": "frontend", "device_id": "node-current", "job_id": "job-1", "amount": 4.0, "currency": "credits", "created_at": "3"}
            ]
        });
        let rotated = super::sync_local_credit_log(
            &rotated_payload,
            "node-current",
            0,
            25,
            1_000 + super::CREDIT_LOG_WINDOW_MILLIS + 1,
        )
        .expect("rotated sync");
        assert_eq!(rotated.appended_entries, 1);
        assert_eq!(super::credit_log_files().len(), 2);
        assert_eq!(
            rotated.current_entries[0]["ledger"]["id"].as_str(),
            Some("frontend")
        );

        std::env::remove_var("OPENGPU_HOME");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn onboarding_complete_hint_requires_a_contribution_cap() {
        let config = Config::default();

        let hint = super::onboarding_completion_hint(&config, true, Some("Qwen"), true);

        assert_eq!(hint, "run `opengpu cap` to choose the contribution budget");
    }

    #[test]
    fn onboarding_complete_hint_requires_start_when_not_connected() {
        let mut config = Config::default();
        config.contribution_percent = 30;

        let hint = super::onboarding_completion_hint(&config, true, Some("Qwen"), true);

        assert_eq!(hint, "run `opengpu start` to bring the node online");
    }
}
