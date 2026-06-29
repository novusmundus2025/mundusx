use crate::contracts::{
    Backend, ModelCapability, WorkerHealthReport, WorkerLaunchRequest, WorkerLaunchResponse,
    WorkerPolicyReport,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser, Debug)]
#[command(
    name = "opengpu-agent worker",
    version,
    about = "MundusX local worker process",
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
    pub system_prompt: Option<String>,
    #[arg(long)]
    pub max_tokens: Option<u32>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub top_p: Option<f32>,
    #[arg(long)]
    pub seed: Option<u64>,
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

#[derive(Debug, Clone, Default)]
pub struct CudaDiagnostics {
    pub device_available: bool,
    pub driver_available: bool,
    pub device_name: Option<String>,
    pub memory_mb: Option<u32>,
    pub low_vram_profile: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrustedExecutable {
    pub path: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TrustedRuntimePaths {
    #[serde(default)]
    pub llama_cli: Option<TrustedExecutable>,
    #[serde(default)]
    pub nvidia_smi: Option<TrustedExecutable>,
}

#[derive(Debug, Deserialize)]
struct CachedModelRecord {
    name: String,
    active: bool,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    quantization: Option<String>,
    #[serde(default)]
    size_bytes: Option<u64>,
    #[serde(default)]
    estimated_vram_mb: Option<u64>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    compatibility_reason: Option<String>,
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

fn config_dir() -> PathBuf {
    env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn trusted_runtime_paths_path() -> PathBuf {
    env::var_os("OPENGPU_TRUSTED_RUNTIME_PATHS")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir().join("trusted-runtime-paths.json"))
}

fn load_trusted_runtime_paths() -> Result<Option<TrustedRuntimePaths>, String> {
    let path = trusted_runtime_paths_path();
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let config = serde_json::from_str::<TrustedRuntimePaths>(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    Ok(Some(config))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buffer[..read]);
    }

    Ok(format!("{:x}", sha2::Digest::finalize(hasher)))
}

fn verify_trusted_executable(name: &str, pinned: &TrustedExecutable) -> Result<PathBuf, String> {
    let path = PathBuf::from(&pinned.path);
    if !path.is_absolute() {
        return Err(format!(
            "untrusted {name}: pinned runtime path must be absolute"
        ));
    }
    if !path.is_file() {
        return Err(format!(
            "missing {name}: trusted runtime path {} does not exist",
            path.display()
        ));
    }
    if let Some(expected) = pinned.sha256.as_deref() {
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(format!(
                "untrusted {name}: trusted runtime hash changed for {}",
                path.display()
            ));
        }
    }

    Ok(path)
}

fn trusted_runtime_executable(name: &str) -> Result<PathBuf, String> {
    let trusted = load_trusted_runtime_paths()?;
    let pinned = trusted.as_ref().and_then(|paths| match name {
        "llama-cli" => paths.llama_cli.as_ref(),
        "nvidia-smi" => paths.nvidia_smi.as_ref(),
        _ => None,
    });

    match pinned {
        Some(pinned) => verify_trusted_executable(name, pinned),
        None => Ok(PathBuf::from(name)),
    }
}

fn imported_model_path_from_cache(model_dir: &Path, model_name: Option<&str>) -> Option<PathBuf> {
    let manifest_dir = model_dir.join(".opengpu");
    let entries = fs::read_dir(manifest_dir).ok()?;
    let mut active_fallback = None;

    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let raw = fs::read_to_string(entry.path()).ok()?;
            let record = serde_json::from_str::<CachedModelRecord>(&raw).ok()?;
            let path = record
                .source_path
                .as_ref()
                .map(PathBuf::from)
                .filter(|path| path.is_file());

            if model_name
                .map(|name| record.name == name)
                .unwrap_or(record.active)
            {
                if path.is_some() {
                    return path;
                }
            } else if record.active {
                active_fallback = path;
            }
        }
    }

    active_fallback
}

pub fn active_model_capability(
    model_dir: &Path,
    model_name: Option<&str>,
) -> Option<ModelCapability> {
    let manifest_dir = model_dir.join(".opengpu");
    let entries = fs::read_dir(manifest_dir).ok()?;
    let mut active_fallback = None;

    for entry in entries.flatten() {
        if entry.file_type().ok()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("json")
        {
            let raw = fs::read_to_string(entry.path()).ok()?;
            let record = serde_json::from_str::<CachedModelRecord>(&raw).ok()?;
            let capability = ModelCapability {
                name: record.name.clone(),
                path: record.source_path.clone(),
                format: record.format.clone(),
                quantization: record.quantization.clone(),
                size_bytes: record.size_bytes,
                estimated_vram_mb: record.estimated_vram_mb,
                compatibility: record.compatibility.clone(),
                compatibility_reason: record.compatibility_reason.clone(),
            };

            if model_name
                .map(|name| record.name == name)
                .unwrap_or(record.active)
            {
                return Some(capability);
            }

            if record.active {
                active_fallback = Some(capability);
            }
        }
    }

    active_fallback
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
    if let Some(path) = imported_model_path_from_cache(model_dir, model_name) {
        return Ok(path);
    }

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
    let llama_cli = trusted_runtime_executable("llama-cli")?;
    let output = Command::new(&llama_cli)
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

fn probe_llama_cli_available() -> Result<(), String> {
    let llama_cli = trusted_runtime_executable("llama-cli")?;
    let output = Command::new(&llama_cli)
        .arg("--version")
        .output()
        .map_err(|error| format!("failed to launch llama-cli: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "llama-cli --version exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr.lines().next().unwrap_or("no stderr")
        ));
    }

    Ok(())
}

fn parse_nvidia_smi_query(stdout: &str) -> CudaDiagnostics {
    let mut best: Option<(String, u32)> = None;

    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((name, memory)) = line.rsplit_once(',') else {
            continue;
        };
        let Some(memory_mb) = memory.trim().parse::<u32>().ok() else {
            continue;
        };
        let name = name.trim().to_string();
        if best
            .as_ref()
            .map(|(_, best_memory)| memory_mb > *best_memory)
            .unwrap_or(true)
        {
            best = Some((name, memory_mb));
        }
    }

    if let Some((device_name, memory_mb)) = best {
        let low_vram_profile = memory_mb <= 4096;
        let mut notes = Vec::new();
        if low_vram_profile {
            notes.push(format!(
                "CUDA low-VRAM profile selected for {memory_mb} MB; advertise modest workloads only"
            ));
        }

        CudaDiagnostics {
            device_available: true,
            driver_available: true,
            device_name: Some(device_name),
            memory_mb: Some(memory_mb),
            low_vram_profile,
            notes,
        }
    } else {
        CudaDiagnostics {
            driver_available: true,
            notes: vec!["nvidia-smi returned no parseable GPU rows".to_string()],
            ..CudaDiagnostics::default()
        }
    }
}

pub fn probe_cuda_diagnostics() -> CudaDiagnostics {
    let nvidia_smi = match trusted_runtime_executable("nvidia-smi") {
        Ok(path) => path,
        Err(error) => {
            return CudaDiagnostics {
                notes: vec![error],
                ..CudaDiagnostics::default()
            };
        }
    };
    let output = Command::new(&nvidia_smi)
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_nvidia_smi_query(&stdout)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            CudaDiagnostics {
                notes: vec![format!(
                    "nvidia-smi exited {}: {}",
                    output.status.code().unwrap_or(-1),
                    stderr.lines().next().unwrap_or("no stderr")
                )],
                ..CudaDiagnostics::default()
            }
        }
        Err(error) => CudaDiagnostics {
            notes: vec![format!(
                "nvidia-smi unavailable; install NVIDIA driver/CUDA runtime first: {error}"
            )],
            ..CudaDiagnostics::default()
        },
    }
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
    backend: Backend,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    seed: u64,
) -> Result<(String, String), String> {
    let llama_cli = trusted_runtime_executable("llama-cli")?;
    let runtime_mode = match backend {
        Backend::Cuda => "cuda",
        _ => "blas",
    };
    let device = match backend {
        Backend::Cuda => "CUDA",
        _ => "BLAS",
    };
    let mut command = Command::new(&llama_cli);
    command
        .arg("-m")
        .arg(model_path)
        .arg("--device")
        .arg(device)
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
        .arg(max_tokens.to_string())
        .arg("--temp")
        .arg(temperature.to_string())
        .arg("--top-p")
        .arg(top_p.to_string())
        .arg("--seed")
        .arg(seed.to_string());

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

    Ok((generated, runtime_mode.to_string()))
}

pub fn probe_worker_health(
    model_dir: &Path,
    model_name: Option<&str>,
    backend: Backend,
) -> WorkerHealthReport {
    let mut notes = Vec::new();
    let mut model_path = None;
    let mut llama_cli_available = false;
    let mut blas_device_available = false;
    let power_state = probe_power_state();
    let cuda = probe_cuda_diagnostics();

    match resolve_model_path(model_dir, model_name) {
        Ok(path) => model_path = Some(path.display().to_string()),
        Err(error) => notes.push(format!("model cache missing: {error}")),
    }

    match backend {
        Backend::Cuda => match probe_llama_cli_available() {
            Ok(()) => llama_cli_available = true,
            Err(error) => notes.push(error),
        },
        _ => match probe_llama_cli_devices() {
            Ok(stdout) => {
                llama_cli_available = true;
                blas_device_available = stdout.lines().any(|line| line.contains("BLAS"));
                if !blas_device_available {
                    notes.push("BLAS device not listed by llama-cli".to_string());
                }
            }
            Err(error) => notes.push(error),
        },
    }

    if backend == Backend::Cuda {
        notes.extend(cuda.notes.clone());
        if !cuda.device_available {
            notes.push(
                "CUDA device not detected; node will stay unavailable for CUDA jobs".to_string(),
            );
        }
    }

    let healthy = if backend == Backend::Cuda {
        model_path.is_some()
            && llama_cli_available
            && cuda.device_available
            && cuda.driver_available
    } else {
        model_path.is_some() && llama_cli_available && blas_device_available
    };

    let runtime_mode = if backend == Backend::Cuda {
        "cuda".to_string()
    } else {
        "blas".to_string()
    };
    let supported_runtime_modes = if healthy {
        vec!["local".to_string()]
    } else {
        Vec::new()
    };

    WorkerHealthReport {
        healthy,
        model_dir: model_dir.display().to_string(),
        model_name: model_name.map(|name| name.to_string()),
        model_path,
        llama_cli_available,
        blas_device_available,
        cuda_device_available: cuda.device_available,
        cuda_driver_available: cuda.driver_available,
        cuda_device_name: cuda.device_name,
        cuda_memory_mb: cuda.memory_mb,
        cuda_low_vram_profile: cuda.low_vram_profile,
        power_source: power_state.source,
        on_battery: power_state.on_battery,
        battery_percent: power_state.battery_percent,
        runtime_mode,
        supported_runtime_modes,
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

fn run_llama_request(
    request: &WorkerLaunchRequest,
    backend: Backend,
) -> Result<WorkerLaunchResponse, String> {
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

    let system_prompt = request.system_prompt.as_deref().unwrap_or("").trim();
    let prompt = if system_prompt.is_empty() {
        request.prompt.clone()
    } else {
        format!("System:\n{system_prompt}\n\nUser:\n{}", request.prompt)
    };
    let max_tokens = request.max_tokens.unwrap_or(16).max(1);
    let temperature = request.temperature.unwrap_or(0.2).max(0.0);
    let top_p = request.top_p.unwrap_or(0.9).clamp(0.0, 1.0);
    let seed = request.seed.unwrap_or(42);

    let (generated, runtime_mode) = run_llama_command(
        &model_path,
        &prompt,
        backend,
        max_tokens,
        temperature,
        top_p,
        seed,
    )?;

    Ok(WorkerLaunchResponse {
        job_id: request.job_id.clone(),
        worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
        status: "completed".to_string(),
        output: format!(
            "llama.cpp mode={runtime_mode}; model={model_name}; path={}; max_tokens={max_tokens}; temperature={temperature}; top_p={top_p}; seed={seed}; response={generated}",
            model_path.display(),
        ),
        error: None,
        backend,
        node_id: request.node_id.clone(),
        model: Some(model_name),
        runtime_mode: Some(runtime_mode),
    })
}

fn execute_request(request: &WorkerLaunchRequest) -> WorkerLaunchResponse {
    let backend = resolved_backend(request.backend);
    if matches!(backend, Backend::M | Backend::Cuda) {
        return match run_llama_request(request, backend) {
            Ok(response) => response,
            Err(error) => WorkerLaunchResponse {
                job_id: request.job_id.clone(),
                worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
                status: "failed".to_string(),
                output: String::new(),
                error: Some(error),
                backend,
                node_id: request.node_id.clone(),
                model: request.model.clone(),
                runtime_mode: Some(backend.as_str().to_string()),
            },
        };
    }

    WorkerLaunchResponse {
        job_id: request.job_id.clone(),
        worker_id: format!("worker-{}", uuid::Uuid::new_v4().simple()),
        status: "failed".to_string(),
        output: String::new(),
        error: Some(format!(
            "backend {backend} is not enabled in the Mac M-only worker"
        )),
        backend,
        node_id: request.node_id.clone(),
        model: request.model.clone(),
        runtime_mode: Some(backend.as_str().to_string()),
    }
}

pub fn worker_main(cli: WorkerCli) {
    let request = WorkerLaunchRequest {
        job_id: cli.job_id,
        node_id: cli.node_id,
        backend: cli.backend,
        prompt: cli.prompt,
        model: cli.model,
        system_prompt: cli.system_prompt,
        max_tokens: cli.max_tokens,
        temperature: cli.temperature,
        top_p: cli.top_p,
        seed: cli.seed,
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
    if let Some(system_prompt) = request.system_prompt.as_ref() {
        command.arg("--system-prompt").arg(system_prompt);
    }
    if let Some(max_tokens) = request.max_tokens {
        command.arg("--max-tokens").arg(max_tokens.to_string());
    }
    if let Some(temperature) = request.temperature {
        command.arg("--temperature").arg(temperature.to_string());
    }
    if let Some(top_p) = request.top_p {
        command.arg("--top-p").arg(top_p.to_string());
    }
    if let Some(seed) = request.seed {
        command.arg("--seed").arg(seed.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn with_temp_runtime_home(test: impl FnOnce(&Path)) {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-agent-runtime-paths-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let previous_home = env::var_os("OPENGPU_HOME");
        let previous_paths = env::var_os("OPENGPU_TRUSTED_RUNTIME_PATHS");
        env::set_var("OPENGPU_HOME", &temp_dir);
        env::set_var(
            "OPENGPU_TRUSTED_RUNTIME_PATHS",
            temp_dir.join("trusted-runtime-paths.json"),
        );

        test(&temp_dir);

        match previous_home {
            Some(value) => env::set_var("OPENGPU_HOME", value),
            None => env::remove_var("OPENGPU_HOME"),
        }
        match previous_paths {
            Some(value) => env::set_var("OPENGPU_TRUSTED_RUNTIME_PATHS", value),
            None => env::remove_var("OPENGPU_TRUSTED_RUNTIME_PATHS"),
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    fn write_trusted_paths(home: &Path, paths: TrustedRuntimePaths) {
        fs::write(
            home.join("trusted-runtime-paths.json"),
            serde_json::to_string_pretty(&paths).expect("trusted paths json"),
        )
        .expect("trusted paths");
    }

    #[test]
    fn parses_low_vram_cuda_device_from_nvidia_smi() {
        let diagnostics = parse_nvidia_smi_query("NVIDIA GeForce GTX 1050 Ti, 4096\n");

        assert!(diagnostics.device_available);
        assert!(diagnostics.driver_available);
        assert_eq!(
            diagnostics.device_name.as_deref(),
            Some("NVIDIA GeForce GTX 1050 Ti")
        );
        assert_eq!(diagnostics.memory_mb, Some(4096));
        assert!(diagnostics.low_vram_profile);
        assert!(diagnostics
            .notes
            .iter()
            .any(|note| note.contains("low-VRAM profile")));
    }

    #[test]
    fn chooses_largest_cuda_device_from_nvidia_smi() {
        let diagnostics =
            parse_nvidia_smi_query("NVIDIA GeForce GTX 1050 Ti, 4096\nNVIDIA RTX 4090, 24564\n");

        assert_eq!(diagnostics.device_name.as_deref(), Some("NVIDIA RTX 4090"));
        assert_eq!(diagnostics.memory_mb, Some(24564));
        assert!(!diagnostics.low_vram_profile);
    }

    #[test]
    fn resolves_imported_model_source_path_from_manifest() {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-agent-imported-model-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let manifest_dir = temp_dir.join(".opengpu");
        fs::create_dir_all(&manifest_dir).expect("manifest dir");
        let source_path = temp_dir.join("external-q4_k_m.gguf");
        fs::write(&source_path, b"model").expect("model file");
        fs::write(
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

        let resolved = resolve_model_path(&temp_dir, Some("external")).expect("resolve model");

        assert_eq!(resolved, source_path);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn cuda_worker_uses_llama_runtime_instead_of_m_only_rejection() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: home.join("missing-llama-cli").display().to_string(),
                        sha256: None,
                    }),
                    nvidia_smi: None,
                },
            );
            let previous_model_dir = env::var_os("OPENGPU_MODEL_DIR");
            env::set_var("OPENGPU_MODEL_DIR", &model_dir);

            let response = execute_request(&WorkerLaunchRequest {
                job_id: "job-1".to_string(),
                node_id: "node-1".to_string(),
                backend: Backend::Cuda,
                prompt: "hello".to_string(),
                model: Some("qwen".to_string()),
                system_prompt: None,
                max_tokens: Some(4),
                temperature: Some(0.2),
                top_p: Some(0.9),
                seed: Some(42),
            });

            match previous_model_dir {
                Some(value) => env::set_var("OPENGPU_MODEL_DIR", value),
                None => env::remove_var("OPENGPU_MODEL_DIR"),
            }

            assert_eq!(response.backend, Backend::Cuda);
            assert_eq!(response.runtime_mode.as_deref(), Some("cuda"));
            let error = response.error.expect("missing llama-cli error");
            assert!(error.contains("missing llama-cli"));
            assert!(!error.contains("Mac M-only worker"));
        });
    }

    #[test]
    fn cuda_health_requires_llama_runtime_before_advertising_local_support() {
        with_temp_runtime_home(|home| {
            let model_dir = home.join("models");
            let model_cache = model_dir.join("qwen");
            fs::create_dir_all(&model_cache).expect("model dir");
            fs::write(model_cache.join("model.gguf"), b"model").expect("model file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: home.join("missing-llama-cli").display().to_string(),
                        sha256: None,
                    }),
                    nvidia_smi: None,
                },
            );

            let health = probe_worker_health(&model_dir, Some("qwen"), Backend::Cuda);

            assert!(!health.healthy);
            assert!(!health.llama_cli_available);
            assert_eq!(
                health.model_path.as_deref(),
                Some(model_cache.join("model.gguf").to_str().unwrap())
            );
            assert!(health.supported_runtime_modes.is_empty());
            assert!(health
                .notes
                .iter()
                .any(|note| note.contains("missing llama-cli")));
        });
    }

    #[test]
    fn trusted_runtime_uses_pinned_absolute_path() {
        with_temp_runtime_home(|home| {
            let runtime = home.join("trusted-llama-cli");
            fs::write(&runtime, b"trusted runtime").expect("runtime file");
            let digest = sha256_file(&runtime).expect("runtime hash");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: runtime.display().to_string(),
                        sha256: Some(digest),
                    }),
                    nvidia_smi: None,
                },
            );

            let resolved = trusted_runtime_executable("llama-cli").expect("trusted runtime");

            assert_eq!(resolved, runtime);
        });
    }

    #[test]
    fn trusted_runtime_rejects_hash_change() {
        with_temp_runtime_home(|home| {
            let runtime = home.join("trusted-llama-cli");
            fs::write(&runtime, b"trusted runtime").expect("runtime file");
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: runtime.display().to_string(),
                        sha256: Some("0".repeat(64)),
                    }),
                    nvidia_smi: None,
                },
            );

            let error = trusted_runtime_executable("llama-cli").expect_err("hash must fail");

            assert!(error.contains("untrusted llama-cli"));
            assert!(error.contains("hash changed"));
        });
    }

    #[test]
    fn trusted_runtime_rejects_relative_path_lookup() {
        with_temp_runtime_home(|home| {
            write_trusted_paths(
                home,
                TrustedRuntimePaths {
                    llama_cli: Some(TrustedExecutable {
                        path: "llama-cli".to_string(),
                        sha256: None,
                    }),
                    nvidia_smi: None,
                },
            );

            let error = trusted_runtime_executable("llama-cli").expect_err("relative path");

            assert!(error.contains("pinned runtime path must be absolute"));
        });
    }
}
