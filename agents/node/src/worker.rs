use crate::contracts::{Backend, WorkerLaunchRequest, WorkerLaunchResponse};
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
        .arg("--color")
        .arg("off")
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg("64")
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

    let generated = String::from_utf8(output.stdout)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();

    Ok((generated, "blas".to_string()))
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
