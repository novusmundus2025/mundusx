mod auth_token;
mod config;
mod identity;
mod model;
mod model_catalog;
mod routing;
mod theme;
mod types;

use clap::{Parser, Subcommand};
use crossterm::event::{read, Event, KeyCode, KeyModifiers};
use crossterm::style::Color;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use serde::Serialize;
use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use types::Backend;

use config::{config_exists, load_config, resolved_config_path, save_config, Config};
use identity::{device_id_for_identity, ensure_identity, load_identity, load_or_create_identity};
use model::{
    active_model_name, add_model, configured_model_dir_string, ensure_catalog_model_fits,
    ensure_effective_model_dir, import_model, list_models, prune_models, remove_model, use_model,
    ImportModelOptions, ModelRecord,
};
use model_catalog::{
    selectable_catalog_options_for, selectable_options_for, selection_for, ModelOption,
};

const PUBLIC_CONTROL_PLANE_URL: &str = "https://api.mundusx.ai";

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
    },
    /// Start the MundusX network
    Start {
        /// Run the node agent in the foreground with logs for debugging
        #[arg(long)]
        debug: bool,
    },
    /// Join the MundusX network (boots local state on first use)
    Connect,
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
    /// Update the MundusX binary
    Update,
    /// Show earned credits from the control plane
    Credits {
        #[arg(long)]
        json: bool,
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
    /// Run an inference request — tries local worker first, falls back to network
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
        /// Maximum tokens to generate
        #[arg(long, default_value_t = 512)]
        max_tokens: u32,
        /// Maximum seconds to wait for a remote fallback job
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
        /// Maximum tokens to generate
        #[arg(long, default_value_t = 512)]
        max_tokens: u32,
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
        .unwrap_or_else(|| "unset".to_string())
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

fn doctor_payload(config: &Config) -> serde_json::Value {
    let resolved_path = resolved_config_path();
    let local_path = config::local_config_path();
    let home_path = config::config_path();
    let config_dir = config::config_dir();
    let model_dir = model::effective_model_dir(config);
    let backend = resolved_backend(config);
    let cuda = live_cuda_doctor_payload(backend);

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
        "active_model": active_model_name(config),
        "auth_token_present": auth_token::operator_token_present(config),
        "cuda": cuda,
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
    Config {
        device_id: device_id_for_identity(identity),
        public_key_fingerprint: Some(identity.fingerprint.clone()),
        ..Config::default()
    }
}

// ---------------------------------------------------------------------------
// Local-first inference
// ---------------------------------------------------------------------------

struct InferenceResult {
    output: String,
    node_label: String,
    model_name: Option<String>,
    job_id: Option<String>,
    status: Option<String>,
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct LocalModelManifestRecord {
    name: String,
    active: bool,
    source_path: Option<String>,
}

/// Try running inference locally via llama-cli, then fall back to the
/// control plane if local is unavailable.
fn run_inference_local_first(
    config: &Config,
    prompt: &str,
    model: Option<&str>,
    backend: crate::types::Backend,
    max_tokens: u32,
    timeout_secs: u64,
    interval_secs: u64,
) -> Result<InferenceResult, String> {
    // --- 1. try local ---------------------------------------------------------
    // Honour OPENGPU_MODEL_DIR env var as an override (useful for pointing at
    // LM Studio or other external model directories without changing config).
    let model_dir = std::env::var_os("OPENGPU_MODEL_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| model::effective_model_dir(config));
    match run_local_inference(&model_dir, prompt, model, max_tokens) {
        Ok((output, model_name)) => {
            return Ok(InferenceResult {
                output,
                node_label: "local".to_string(),
                model_name,
                job_id: None,
                status: Some("completed".to_string()),
                error: None,
            });
        }
        Err(local_err) => {
            theme::warn(format!("local worker unavailable: {local_err}"));
            theme::warn("falling back to network routing...");
        }
    }

    // --- 2. remote fallback via control plane ---------------------------------
    let (request_id, job) = build_job_submission_payload(prompt, model, backend, max_tokens);
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
            Ok(InferenceResult {
                output,
                node_label: "network".to_string(),
                model_name: model.map(|m| m.to_string()),
                job_id: Some(job_id),
                status: Some(status),
                error: completed
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
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
) -> (String, serde_json::Value) {
    let request_id = format!("req-{}", uuid::Uuid::new_v4().simple());
    let job = serde_json::json!({
        "request_id": request_id,
        "prompt": prompt,
        "preferred_backend": backend.as_str(),
        "model": model,
        "max_tokens": max_tokens,
    });
    (request_id, job)
}

fn submit_job(
    config: &Config,
    prompt: &str,
    model: Option<&str>,
    backend: Backend,
    max_tokens: u32,
) -> Result<serde_json::Value, String> {
    let (_, job) = build_job_submission_payload(prompt, model, backend, max_tokens);
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

fn remote_job_output(payload: &serde_json::Value) -> Option<String> {
    ["output", "final_output", "result"]
        .iter()
        .filter_map(|field| payload.get(*field))
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

fn wait_for_job(
    config: &Config,
    job_id: &str,
    timeout_secs: u64,
    interval_secs: u64,
) -> Result<serde_json::Value, String> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let interval = Duration::from_secs(interval_secs.max(1));

    loop {
        let payload = get_job(config, job_id)?;
        if job_is_terminal(&payload) {
            return Ok(payload);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for job {job_id} while status was {}",
                job_state(&payload)
            ));
        }
        thread::sleep(interval);
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
    Ok(())
}

/// Find a GGUF model file under `model_dir`, optionally matching `model_name`,
/// then invoke `llama-cli` and return (output, model_name).
fn run_local_inference(
    model_dir: &std::path::Path,
    prompt: &str,
    model_name: Option<&str>,
    max_tokens: u32,
) -> Result<(String, Option<String>), String> {
    // resolve model path
    let model_path = resolve_local_model_path(model_dir, model_name)
        .map_err(|error| format!("no cached model: {error}"))?;

    let detected_name = model_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string());

    // check llama-cli is available
    let which = Command::new("llama-cli")
        .arg("--version")
        .output()
        .map_err(|_| "llama-cli not found in PATH".to_string())?;
    if !which.status.success() {
        return Err("llama-cli --version failed".to_string());
    }

    // run inference
    let output = Command::new("llama-cli")
        .arg("-m")
        .arg(&model_path)
        .arg("--device")
        .arg("BLAS")
        .arg("--simple-io")
        .arg("--single-turn")
        .arg("--no-display-prompt")
        .arg("--no-perf")
        .arg("--log-disable")
        .arg("--color")
        .arg("off")
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg(max_tokens.to_string())
        .arg("--seed")
        .arg("42")
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no output")
        ));
    }

    let transcript = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();

    Ok((transcript, detected_name))
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
        return Some("secure device identity is unavailable".to_string());
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

fn provider_count(
    config: &Config,
    power: &PowerState,
    active_model: Option<&str>,
    identity_ready: bool,
) -> usize {
    if config.connected
        && !config.paused
        && policy_allowed(config, power, active_model, identity_ready)
    {
        1
    } else {
        0
    }
}

fn print_config_summary(config: &Config, path: &std::path::Path) {
    let detected_backend = resolved_backend(config);
    let power = probe_power_state();
    let active_model = active_model_name(config);
    let identity_ready = identity_ready();
    let allowed = policy_allowed(config, &power, active_model.as_deref(), identity_ready);
    let provider_count = provider_count(config, &power, active_model.as_deref(), identity_ready);
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
    theme::field("identityReady", theme::boolean(identity_ready, "yes", "no"));
    theme::field("identityTrustPath", identity::trust_path());
    theme::field("providerCount", provider_count);
    theme::field("modelDir", configured_model_dir_string(config));
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
    let active_model = active_model_name(config);
    let identity_ready = identity_ready();
    let allowed = policy_allowed(config, &power, active_model.as_deref(), identity_ready);

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
    theme::field("identityReady", theme::boolean(identity_ready, "yes", "no"));
    theme::field("identityTrustPath", identity::trust_path());
    theme::field("modelDir", configured_model_dir_string(config));
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

fn stop_process_by_pid(pid: u32) -> Result<(), String> {
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
        Ok(())
    } else {
        Err(format!("agent stop command exited with {status}"))
    }
}

fn stop_background_node_agent() -> Result<Option<u32>, String> {
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

    let result = stop_process_by_pid(pid);
    remove_node_agent_pid();
    result.map(|_| Some(pid))
}

fn launch_node_agent(debug: bool) -> Result<(), String> {
    let agent = resolve_node_agent_executable();
    let mut command = Command::new(&agent);
    command.arg("run");

    if let Err(error) = stop_background_node_agent() {
        eprintln!("agentStopWarning: {error}");
    }

    if debug {
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        println!("agentMode: foreground debug");
        println!("agentCommand: {} run", agent.display());
        let status = command
            .status()
            .map_err(|error| format!("failed to run node agent `{}`: {error}", agent.display()))?;
        if status.success() {
            return Ok(());
        }
        return Err(format!("node agent exited with {status}"));
    }

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|error| format!("failed to start node agent `{}`: {error}", agent.display()))?;
    let pid = child.id();
    if let Err(error) = write_node_agent_pid(pid) {
        let _ = stop_process_by_pid(pid);
        return Err(error);
    }
    println!("agent: started");
    println!("agentPid: {}", pid);
    println!("agentMode: background");
    Ok(())
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
    let active_model = active_model_name(config);
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
        Backend::Auto => "automatic routing budget",
    }
}

fn default_contribution_percent(backend: Backend) -> u8 {
    match backend {
        Backend::Cuda => 30,
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
        ("linux", "x86_64", Backend::Cuda) => "linux-x86_64-cuda",
        ("linux", "aarch64", Backend::Cuda) => "linux-aarch64-cuda",
        ("windows", "x86_64", _) => "windows-x86_64-generic",
        ("linux", "x86_64", _) => "linux-x86_64-generic",
        ("linux", "aarch64", _) => "linux-aarch64-generic",
        ("macos", "x86_64", _) => "macos-x86_64-generic",
        _ => "unsupported-or-generic",
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
    let backend = detect_backend();
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

fn contribution_vram_budget_mb(
    total_vram_mb: Option<u64>,
    contribution_percent: u8,
) -> Option<u64> {
    total_vram_mb.map(|total| total.saturating_mul(contribution_percent as u64) / 100)
}

fn model_vram_budget_mb(config: &Config, backend: Backend) -> Option<u64> {
    if backend != Backend::Cuda {
        return None;
    }

    contribution_vram_budget_mb(detect_cuda_vram_mb(), config.contribution_percent)
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
    Cancelled,
}

enum ControlPlaneChoice {
    Public,
    Private,
}

fn prompt_control_plane_choice() -> ControlPlaneChoice {
    const OPTIONS: &[(&str, &str)] = &[
        ("Public MundusX", "use the hosted mundusx.ai control plane"),
        ("Private / custom", "enter your own control-plane URL"),
    ];

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return ControlPlaneChoice::Public;
    }

    let mut selected = 0usize;
    if enable_raw_mode().is_err() {
        return ControlPlaneChoice::Public;
    }

    let render_menu = |selected: usize| {
        print!("\x1b[2J\x1b[H");
        println!("Which control plane should this node use?");
        println!("-----------------------------------------");
        for (index, (label, detail)) in OPTIONS.iter().enumerate() {
            let marker = if index == selected { ">>" } else { "  " };
            println!("{marker} {label} - {detail}");
        }
        println!();
        println!("Use ↑/↓ and Enter");
        let _ = io::stdout().flush();
    };

    render_menu(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    eprintln!("cancelled");
                    std::process::exit(130);
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    render_menu(selected);
                }
                KeyCode::Down => {
                    if selected + 1 < OPTIONS.len() {
                        selected += 1;
                    }
                    render_menu(selected);
                }
                KeyCode::Enter => break selected,
                KeyCode::Esc => break 0,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break 0,
        }
    };

    let _ = disable_raw_mode();
    if result == 1 {
        ControlPlaneChoice::Private
    } else {
        ControlPlaneChoice::Public
    }
}

fn read_private_control_plane_url() -> String {
    loop {
        print!("Private control-plane URL [blank for public MundusX]: ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            eprintln!("failed to read control-plane URL");
            std::process::exit(1);
        }
        let url = input.trim();
        if url.is_empty() {
            return PUBLIC_CONTROL_PLANE_URL.to_string();
        }
        if !url.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
            return url.to_string();
        }
        println!("Enter a full URL, for example http://127.0.0.1:8787, or leave blank for public MundusX");
    }
}

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
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return PUBLIC_CONTROL_PLANE_URL.to_string();
        }
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return trimmed.to_string();
        }
        eprintln!("control-plane URL must start with http:// or https://");
        std::process::exit(2);
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

fn prompt_contribution_percent(default_percent: u8) -> PromptOutcome {
    const OPTIONS: &[Option<(u8, &str)>] = &[
        Some((20, "light")),
        Some((30, "balanced")),
        Some((50, "strong")),
        Some((65, "high")),
        Some((80, "maximum")),
        None,
    ];

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

    let render_menu = |selected: usize| {
        print!("\x1b[2J\x1b[H");
        println!("Contribution level");
        println!("-------------------");
        for (index, option) in OPTIONS.iter().enumerate() {
            let marker = if index == selected { ">>" } else { "  " };
            match option {
                Some((percent, label)) => println!("{marker} {percent:>2}% - {label}"),
                None => println!("{marker} custom - type exact percent (1-80)"),
            }
        }
        println!();
        println!("Use ↑/↓ and Enter");
        let _ = io::stdout().flush();
    };

    render_menu(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    return PromptOutcome::Cancelled;
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    render_menu(selected);
                }
                KeyCode::Down => {
                    if selected + 1 < OPTIONS.len() {
                        selected += 1;
                    }
                    render_menu(selected);
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
    let options = selectable_options_for(backend, gb, available_vram_mb);

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

    let render = |selected: usize| {
        print!("\x1b[2J\x1b[H");
        println!("Which model should this node run?");
        println!(
            "detected: {} / {}GB memory",
            selection.backend, selection.memory_gb
        );
        if let Some(budget) = available_vram_mb {
            println!(
                "model budget: {budget} MB VRAM ({}% contribution cap)",
                config.contribution_percent
            );
        }
        println!("----------------------------------");
        for (i, option) in options.iter().enumerate() {
            let marker = if i == selected { ">>" } else { "  " };
            println!(
                "{marker} {}. {} [{}] — {}",
                i + 1,
                option.label,
                option.name,
                option.notes
            );
        }
        let marker = if selected == options.len() {
            ">>"
        } else {
            "  "
        };
        println!(
            "{marker} {}. Import local GGUF / LM Studio model",
            options.len() + 1
        );
        println!();
        println!("Use ↑/↓ or Tab/Shift+Tab and Enter — you must choose one");
        let _ = io::stdout().flush();
    };

    render(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) => match key.code {
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
                    if selected < options.len() {
                        selected += 1;
                    }
                    render(selected);
                }
                KeyCode::Enter => break selected,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break 1,
        }
    };

    let _ = disable_raw_mode();

    if result == options.len() {
        print!("\x1b[2J\x1b[H");
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

    let render = |selected: usize| {
        print!("\x1b[2J\x1b[H");
        println!(
            "Choose official model to {}",
            if active {
                "download and activate"
            } else {
                "download"
            }
        );
        println!("backend: {}", backend);
        if let Some(budget) = available_vram_mb {
            println!(
                "model budget: {budget} MB VRAM ({}% contribution cap)",
                config.contribution_percent
            );
        }
        println!("----------------------------------");
        for (i, option) in options.iter().enumerate() {
            let marker = if i == selected { ">>" } else { "  " };
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
            println!("{marker} {}. {} [{}]", i + 1, option.label, option.name);
            println!(
                "     provider: {} | fit: ok | backends: {} | {}",
                option.source_kind, backends, estimated
            );
            println!("     url: {}", option.source_url);
        }
        println!();
        println!("Use ↑/↓ or Tab/Shift+Tab and Enter, or press a number — Ctrl-C cancels");
        let _ = io::stdout().flush();
    };

    render(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) => match key.code {
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
) {
    let profile = detect_machine_profile();
    let mut config = if config_exists() {
        current_config_or_default()
    } else {
        match ensure_identity() {
            Ok((identity, _, _)) => config_from_identity(&identity),
            Err(error) => {
                eprintln!("failed to initialize identity: {error}");
                std::process::exit(1);
            }
        }
    };
    config.backend_preference = profile.backend;
    let detected = resolved_backend(&config);
    config.control_plane_url =
        resolve_install_control_plane_url(public, private, control_plane_url);

    let selected_cap = if let Some(value) = cap_percent {
        match normalize_contribution_percent(u16::from(value)) {
            Ok(value) => Some(value),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        println!(
            "contributionQuestion: how much of this {} machine can MundusX use?",
            detected.as_str()
        );
        println!("contributionMeaning: {}", contribution_semantics(detected));
        match prompt_contribution_percent(default_contribution_percent(detected)) {
            PromptOutcome::Selected(value) => Some(value),
            PromptOutcome::Cancelled => None,
        }
    } else if config.contribution_percent == 0 {
        Some(default_contribution_percent(detected))
    } else {
        None
    };

    if let Some(value) = selected_cap {
        config.contribution_percent = value;
    }

    if active_model_name(&config).is_none()
        || io::stdin().is_terminal() && io::stdout().is_terminal()
    {
        let backend = resolved_backend(&config);
        let choice = prompt_model_selection(&config, backend);
        apply_model_choice(&mut config, choice, true);
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
                    "contribution cap: {}",
                    if config.contribution_percent == 0 {
                        "unset".to_string()
                    } else {
                        format!("{}%", config.contribution_percent)
                    }
                ),
                format!(
                    "active model: {}",
                    active_model_name(&config).unwrap_or_else(|| "none".to_string())
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
        }
        Err(error) => {
            eprintln!("failed to save install setup: {error}");
            std::process::exit(1);
        }
    }
}

fn run_start_or_connect(debug: bool) {
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
            eprintln!("failed to load secure device identity: {error}");
            false
        }
    };
    if config.contribution_percent == 0 && io::stdin().is_terminal() && io::stdout().is_terminal() {
        let detected = resolved_backend(&config);
        println!(
            "contributionQuestion: how much of this {} machine can MundusX use?",
            detected.as_str()
        );
        println!("contributionMeaning: {}", contribution_semantics(detected));
        match prompt_contribution_percent(default_contribution_percent(detected)) {
            PromptOutcome::Selected(value) => {
                config.contribution_percent = value;
            }
            PromptOutcome::Cancelled => {
                println!("capHint: run `opengpu cap` before starting contribution");
            }
        }
    }
    if active_model_name(&config).is_none() {
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
            println!(
                "contributionMeaning: {}",
                contribution_semantics(config.backend_preference)
            );
            if config.contribution_percent == 0 {
                println!("capHint: run `opengpu cap` to choose the contribution budget");
            }
            if !identity_ready {
                println!(
                    "identityHint: secure device identity is unavailable; the node is not online yet"
                );
            }
            if !config.onboarding_completed {
                print_onboarding_checklist(&config, &resolved_config_path(), false);
                println!(
                    "onboardingHint: run `opengpu onboarding --complete` after you review the checklist"
                );
            }
            if identity_ready {
                if let Err(error) = launch_node_agent(debug) {
                    eprintln!("{error}");
                    eprintln!(
                        "agentHint: ensure `opengpu-node-agent` is installed beside `opengpu`, or run `opengpu start --debug` for foreground diagnostics"
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
        } => run_install(public, private, control_plane_url, cap_percent),
        Commands::Start { debug } => run_start_or_connect(debug),
        Commands::Connect => run_start_or_connect(false),
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

            let active_model = active_model_name(&config);
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
        Commands::Cap { percent, reset } => {
            if percent.is_some() && reset {
                eprintln!("choose either --percent or --reset, not both");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            let selected = if reset {
                config.contribution_percent = 0;
                None
            } else if let Some(value) = percent {
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
                match prompt_contribution_percent(if config.contribution_percent == 0 {
                    30
                } else {
                    config.contribution_percent
                }) {
                    PromptOutcome::Selected(value) => {
                        config.contribution_percent = value;
                        Some(value)
                    }
                    PromptOutcome::Cancelled => {
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
                    match stop_background_node_agent() {
                        Ok(Some(pid)) => {
                            println!("agent: stopped");
                            println!("agentPid: {pid}");
                        }
                        Ok(None) => println!("agent: not running"),
                        Err(error) => {
                            eprintln!("agentStop: {error}");
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
            let active_model = active_model_name(&config);
            let identity_ready = identity_ready();
            let policy_allowed =
                policy_allowed(&config, &power, active_model.as_deref(), identity_ready);
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
        Commands::Update => {
            println!("updateChannel: localhost preview");
            println!("installPage: http://127.0.0.1:3002/install");
            println!("releasePreview: http://127.0.0.1:8788/releases/latest/download");
            println!("smokeCheck: npm run smoke:local");
        }
        Commands::Run {
            prompt,
            model,
            backend,
            max_tokens,
            timeout,
            interval,
            json,
        } => {
            let config = current_config_or_default();
            match run_inference_local_first(
                &config,
                &prompt,
                model.as_deref(),
                backend,
                max_tokens,
                timeout,
                interval,
            ) {
                Ok(result) => {
                    if json {
                        let output = serde_json::json!({
                            "prompt": prompt,
                            "output": result.output,
                            "node": result.node_label,
                            "model": result.model_name,
                            "job_id": result.job_id,
                            "status": result.status,
                            "error": result.error,
                        });
                        if let Err(error) = print_json(&output) {
                            eprintln!("{error}");
                            std::process::exit(1);
                        }
                    } else {
                        theme::section("Run result");
                        theme::field("node", &result.node_label);
                        if let Some(model_name) = &result.model_name {
                            theme::field("model", model_name);
                        }
                        if let Some(job_id) = &result.job_id {
                            theme::field("jobId", job_id);
                        }
                        if let Some(status) = &result.status {
                            theme::field("status", theme::status(status));
                        }
                        println!();
                        println!("{}", result.output);
                    }
                }
                Err(error) => {
                    eprintln!("run failed: {error}");
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
                    json,
                } => submit_job(&config, &prompt, model.as_deref(), backend, max_tokens)
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
                eprintln!("jobs command failed: {error}");
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
                    config.control_plane_url = url.clone();
                    if let Err(error) = crate::config::save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    println!("controlPlaneUrl: {url}");
                }
            }
        }
        Commands::Credits { json } => {
            let config = current_config_or_default();
            let token = auth_token::effective_operator_token(&config);
            match operator_get_json(&config.control_plane_url, "/v1/credits", token.as_deref()) {
                Ok(payload) => {
                    if json {
                        if let Err(error) = print_json(&payload) {
                            eprintln!("failed to print json: {error}");
                            std::process::exit(1);
                        }
                        return;
                    }
                    let total = payload["total"].as_f64().unwrap_or(0.0);
                    println!("total: {total:.2}");
                    if let Some(by_node) = payload["by_node"].as_object() {
                        if by_node.is_empty() {
                            println!("byNode: none");
                        } else {
                            for (node_id, amount) in by_node {
                                let amount = amount.as_f64().unwrap_or(0.0);
                                println!("  {node_id}: {amount:.2}");
                            }
                        }
                    }
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
        build_job_submission_payload, control_plane_endpoint, cuda_doctor_payload, doctor_payload,
        job_is_terminal, job_status_path, logs_payload, remote_job_output,
        resolve_install_control_plane_url, Cli, Commands, JobsCommands, PUBLIC_CONTROL_PLANE_URL,
    };
    use crate::config::Config;
    use crate::types::Backend;
    use clap::Parser;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn exit_alias_maps_to_disconnect() {
        let cli = Cli::try_parse_from(["opengpu", "exit"]).expect("exit alias should parse");
        assert!(matches!(cli.command, Commands::Disconnect));
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
    fn logs_command_parses() {
        let cli = Cli::try_parse_from(["opengpu", "logs", "--json"]).expect("logs should parse");
        assert!(matches!(cli.command, Commands::Logs { json: true }));
    }

    #[test]
    fn start_debug_flag_parses() {
        let cli = Cli::try_parse_from(["opengpu", "start", "--debug"]).expect("start should parse");
        assert!(matches!(cli.command, Commands::Start { debug: true }));
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
    fn control_plane_endpoint_accepts_https_and_local_http() {
        assert_eq!(
            control_plane_endpoint("https://api.mundusx.ai", "/v1/jobs").as_deref(),
            Ok("https://api.mundusx.ai/v1/jobs")
        );
        assert_eq!(
            control_plane_endpoint("http://127.0.0.1:8787/", "/v1/jobs/job_123").as_deref(),
            Ok("http://127.0.0.1:8787/v1/jobs/job_123")
        );
    }

    #[test]
    fn control_plane_endpoint_rejects_invalid_urls_and_paths() {
        assert!(control_plane_endpoint("api.mundusx.ai", "/v1/jobs").is_err());
        assert!(control_plane_endpoint("ftp://api.mundusx.ai", "/v1/jobs").is_err());
        assert!(control_plane_endpoint("https://api.mundusx.ai", "v1/jobs").is_err());
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
                        json,
                    },
            } => {
                assert_eq!(prompt, "hello");
                assert_eq!(model.as_deref(), Some("smol"));
                assert_eq!(backend, Backend::Cuda);
                assert_eq!(max_tokens, 64);
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
            "--json",
        ])
        .expect("run should parse");

        match cli.command {
            Commands::Run {
                prompt,
                timeout,
                interval,
                json,
                ..
            } => {
                assert_eq!(prompt, "hello");
                assert_eq!(timeout, 45);
                assert_eq!(interval, 3);
                assert!(json);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn job_submission_payload_matches_control_plane_contract() {
        let (_, payload) = build_job_submission_payload("hello", Some("smol"), Backend::Cuda, 64);

        assert!(payload["request_id"]
            .as_str()
            .unwrap_or("")
            .starts_with("req-"));
        assert_eq!(payload["prompt"].as_str(), Some("hello"));
        assert_eq!(payload["model"].as_str(), Some("smol"));
        assert_eq!(payload["preferred_backend"].as_str(), Some("cuda"));
        assert_eq!(payload["max_tokens"].as_u64(), Some(64));
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
            remote_job_output(&serde_json::json!({"status": "completed", "output": null})),
            None
        );
    }

    #[test]
    fn default_contribution_percent_matches_backend_risk() {
        assert_eq!(super::default_contribution_percent(Backend::Cuda), 30);
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
            super::install_profile_for("linux", "x86_64", Backend::Cuda),
            "linux-x86_64-cuda"
        );
        assert_eq!(
            super::install_profile_for("windows", "x86_64", Backend::Auto),
            "windows-x86_64-generic"
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
