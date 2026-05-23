use crate::contracts::{
    Backend, WorkerHealthReport, WorkerLaunchRequest, WorkerLaunchResponse, WorkerPolicyReport,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::env;
use std::path::PathBuf;
use std::path::Path;
use std::process::Command;
use std::io;

#[derive(Parser, Debug)]
#[command(
    name = "opengpu-agent worker",
    version,
    about = "OpenGPU local worker process",
    arg_required_else_help = true
)]
pub struct WorkerCli {
    #[arg(long)]
    pub job_id: String,
    #[arg(long)]
    pub node_id: String,
    #[arg(long)]
    pub prompt: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub backend: Backend,
    #[arg(long)]
    pub json: bool,
}

fn emit_json<T: Serialize>(value: &T) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{payload}");
    Ok(())
}

fn resolved_backend(backend: Backend) -> Backend {
    match backend {
        Backend::Auto => {
            #[cfg(target_os = "macos")]
            {
                if std::env::consts::ARCH == "aarch64" {
                    return Backend::M;
                }
            }

            Backend::Auto
        }
        other => other,
    }
}

fn now_unix_seconds() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[derive(Debug, Clone)]
struct PowerState {
    source: String,
    on_battery: bool,
    battery_percent: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct CachedModelRecord {
    name: String,
    active: bool,
}

fn sanitize_model_name(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }

    let trimmed = output.trim_matches('_');
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

fn active_model_name_from_cache(model_dir: &Path) -> Option<String> {
    let manifest_dir = model_dir.join(".opengpu");
    let entries = fs::read_dir(manifest_dir).ok()?;
    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let raw = fs::read_to_string(entry.path()).ok()?;
            if let Ok(record) = serde_json::from_str::<CachedModelRecord>(&raw) {
                if record.active {
                    return Some(record.name);
                }
            }
        }
    }
    None
}

fn collect_gguf_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_gguf_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("gguf") {
            files.push(path);
        }
    }

    Ok(())
}

fn resolve_model_path(model_dir: &Path, model_name: Option<&str>) -> io::Result<PathBuf> {
    let mut search_dirs = Vec::new();
    if let Some(name) = model_name {
        search_dirs.push(model_dir.join(sanitize_model_name(name)));
    }
    if let Some(active_name) = active_model_name_from_cache(model_dir) {
        let active_dir = model_dir.join(sanitize_model_name(&active_name));
        if !search_dirs.iter().any(|dir| dir == &active_dir) {
            search_dirs.push(active_dir);
        }
    }
    search_dirs.push(model_dir.to_path_buf());

    let mut files = Vec::new();
    for dir in search_dirs {
        collect_gguf_files(&dir, &mut files)?;
        if !files.is_empty() {
            break;
        }
    }

    files.sort();
    files
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no cached GGUF model found"))
}

fn probe_llama_cli_devices() -> Result<String, String> {
    let output = Command::new("llama-cli")
        .arg("--list-devices")
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    let stdout = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli --list-devices exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    Ok(stdout)
}

fn probe_power_state() -> PowerState {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("pmset").args(["-g", "batt"]).output();
        if let Ok(output) = output {
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

fn run_llama_command(
    model_path: &Path,
    prompt: &str,
) -> Result<(String, String), String> {
    let mut command = Command::new("llama-cli");
    command
        .arg("-m")
        .arg(model_path)
        .arg("--device")
        .arg("BLAS")
        .arg("--simple-io")
        .arg("--single-turn")
        .arg("--no-display-prompt")
        .arg("--no-perf")
        .arg("--log-disable")
        .arg("--color")
        .arg("off")
        .arg("--threads")
        .arg("2")
        .arg("--threads-batch")
        .arg("2")
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg("16")
        .arg("--temp")
        .arg("0.2")
        .arg("--seed")
        .arg("42");

    let output = command
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    let transcript = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    let generated = extract_llama_response(prompt, &transcript);

    Ok((generated, "blas".to_string()))
}

pub fn probe_worker_health(model_dir: &Path, model_name: Option<&str>) -> WorkerHealthReport {
    let mut notes = Vec::new();
    let mut model_path = None;
    let mut llama_cli_available = false;
    let mut blas_device_available = false;
    let power_state = probe_power_state();

    match resolve_model_path(model_dir, model_name) {
        Ok(path) => model_path = Some(path.display().to_string()),
        Err(error) => notes.push(format!("model cache missing: {error}")),
    }

    match probe_llama_cli_devices() {
        Ok(stdout) => {
            llama_cli_available = true;
            blas_device_available = stdout.lines().any(|line| line.contains("BLAS"));
            if !blas_device_available {
                notes.push("BLAS device not listed by llama-cli".to_string());
            }
        }
        Err(error) => notes.push(error),
    }

    let healthy = model_path.is_some() && llama_cli_available && blas_device_available;

    WorkerHealthReport {
        healthy,
        model_dir: model_dir.display().to_string(),
        model_name: model_name.map(|name| name.to_string()),
        model_path,
        llama_cli_available,
        blas_device_available,
        power_source: power_state.source,
        on_battery: power_state.on_battery,
        battery_percent: power_state.battery_percent,
        runtime_mode: "blas".to_string(),
        checked_at: now_unix_seconds(),
        notes,
    }
}

pub fn probe_worker_policy(
    health: &WorkerHealthReport,
    contribution_percent: u8,
) -> WorkerPolicyReport {
    let mut notes = Vec::new();
    let mut reason = None;
    let mut allowed = true;
    let recommended_max_contribution_percent;

    if !health.healthy {
        allowed = false;
        reason = Some("worker health is degraded".to_string());
        notes.push("runner health check failed".to_string());
    }

    if contribution_percent == 0 {
        allowed = false;
        reason = Some("contribution percent is unset".to_string());
        notes.push("set a contribution cap before enabling jobs".to_string());
    }

    if health.on_battery {
        recommended_max_contribution_percent = 20;
        if contribution_percent > 20 {
            allowed = false;
            reason = Some("battery power requires contribution percent <= 20".to_string());
            notes.push("plug in the Mac or lower the cap to 20% or less".to_string());
        }
        if let Some(percent) = health.battery_percent {
            if percent <= 20 {
                allowed = false;
                reason = Some("battery too low for active inference".to_string());
                notes.push("battery level is too low to start work safely".to_string());
            }
        }
    } else {
        recommended_max_contribution_percent = 100;
    }

    WorkerPolicyReport {
        allowed,
        reason,
        power_source: health.power_source.clone(),
        on_battery: health.on_battery,
        battery_percent: health.battery_percent,
        recommended_max_contribution_percent,
        checked_at: now_unix_seconds(),
        notes,
    }
}

fn extract_llama_response(prompt: &str, transcript: &str) -> String {
    let mut seen_prompt = false;
    let mut lines = Vec::new();

    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed == format!("> {prompt}") {
            seen_prompt = true;
            continue;
        }

        if seen_prompt {
            if trimmed.starts_with('[')
                || trimmed.starts_with("Exiting")
                || trimmed.starts_with("available commands")
            {
                break;
            }

            if trimmed.starts_with('>') {
                break;
            }

            lines.push(trimmed.to_string());
        }
    }

    if lines.is_empty() {
        transcript
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('[') && !line.starts_with('>'))
            .unwrap_or(transcript)
            .to_string()
    } else {
        lines.join("\n")
    }
}

fn run_llama_request(request: &WorkerLaunchRequest) -> Result<WorkerLaunchResponse, String> {
    let model_dir = env::var_os("OPENGPU_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".opengpu/models")
        });
    let model_path = resolve_model_path(&model_dir, request.model.as_deref())
        .map_err(|error| error.to_string())?;

    let model_name = request
        .model
        .clone()
        .or_else(|| active_model_name_from_cache(&model_dir))
        .unwrap_or_else(|| "active".to_string());

    let (generated, runtime_mode) = run_llama_command(&model_path, &request.prompt)?;

    Ok(WorkerLaunchResponse {
        job_id: request.job_id.clone(),
        worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
        status: "completed".to_string(),
        output: format!(
            "llama.cpp mode={runtime_mode}; model={model_name}; path={}; response={generated}",
            model_path.display()
        ),
        error: None,
        backend: Backend::M,
        node_id: request.node_id.clone(),
    })
}

fn execute_request(request: &WorkerLaunchRequest) -> WorkerLaunchResponse {
    let backend = resolved_backend(request.backend);
    if backend == Backend::M {
        return match run_llama_request(request) {
            Ok(response) => response,
            Err(error) => WorkerLaunchResponse {
                job_id: request.job_id.clone(),
                worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                status: "failed".to_string(),
                output: String::new(),
                error: Some(error),
                backend,
                node_id: request.node_id.clone(),
            },
        };
    }

    WorkerLaunchResponse {
        job_id: request.job_id.clone(),
        worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
        status: "failed".to_string(),
        output: String::new(),
        error: Some(format!("backend {backend} is not enabled in the Mac M-only worker")),
        backend,
        node_id: request.node_id.clone(),
    }
}

pub fn worker_main(cli: WorkerCli) {
    let request = WorkerLaunchRequest {
        job_id: cli.job_id,
        node_id: cli.node_id,
        backend: cli.backend,
        prompt: cli.prompt,
        model: cli.model,
    };

    let response = execute_request(&request);

    if cli.json {
        if let Err(error) = emit_json(&response) {
            eprintln!("failed to print worker json: {error}");
            std::process::exit(1);
        }
        return;
    }

    println!("workerId: {}", response.worker_id);
    println!("jobId: {}", response.job_id);
    println!("nodeId: {}", response.node_id);
    println!("backend: {}", response.backend);
    println!("status: {}", response.status);
    println!("output: {}", response.output);
}

pub fn launch_worker(
    request: &WorkerLaunchRequest,
    model_dir: &Path,
) -> Result<WorkerLaunchResponse, String> {
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    let mut command = Command::new(exe);
    command
        .env("OPENGPU_MODEL_DIR", model_dir)
        .arg("worker")
        .arg("--job-id")
        .arg(&request.job_id)
        .arg("--node-id")
        .arg(&request.node_id)
        .arg("--prompt")
        .arg(&request.prompt)
        .arg("--backend")
        .arg(request.backend.as_str());

    if let Some(model) = request.model.as_ref() {
        command.arg("--model").arg(model);
    }

    let output = command
        .arg("--json")
        .output()
        .map_err(|error| format!("failed to launch worker: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "worker exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    serde_json::from_str(stdout.trim()).map_err(|error| error.to_string())
}
