use crate::contracts::{Backend, WorkerLaunchRequest, WorkerLaunchResponse};
use clap::Parser;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::env;
use std::path::PathBuf;
use std::path::Path;
use std::process::Command;

const METAL_RUNNER_SWIFT: &str = include_str!("metal_runner.swift");

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

            if std::env::var_os("NVIDIA_VISIBLE_DEVICES").is_some()
                || std::env::var_os("CUDA_VISIBLE_DEVICES").is_some()
            {
                return Backend::Cuda;
            }

            Backend::Auto
        }
        other => other,
    }
}

fn normalize_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_lowercase()
}

fn backend_compute_budget(backend: Backend) -> usize {
    match backend {
        Backend::M => 50_000,
        Backend::Cuda => 100_000,
        Backend::Auto => 25_000,
    }
}

fn execute_request_deterministic(request: &WorkerLaunchRequest, backend: Backend) -> WorkerLaunchResponse {
    let worker_id = format!("worker-{}", uuid::Uuid::new_v4().simple());
    let model_name = request.model.clone().unwrap_or_else(|| "default".to_string());
    let budget = backend_compute_budget(backend);
    let tokens: Vec<String> = request
        .prompt
        .split_whitespace()
        .map(normalize_token)
        .filter(|token| !token.is_empty())
        .collect();
    let mut frequencies: BTreeMap<String, usize> = BTreeMap::new();
    for token in &tokens {
        *frequencies.entry(token.clone()).or_insert(0) += 1;
    }

    let mut checksum: u64 = 0;
    for iteration in 0..budget {
        for token in &tokens {
            for byte in token.as_bytes() {
                checksum = checksum
                    .wrapping_mul(31)
                    .wrapping_add((*byte as u64) + iteration as u64 + model_name.len() as u64);
            }
        }
    }

    let unique_tokens = frequencies.len();
    let token_count = tokens.len();
    let top_token = frequencies
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(token, count)| format!("{token} ({count})"))
        .unwrap_or_else(|| "none".to_string());

    let output = format!(
        "backend={backend}; model={model_name}; tokens={token_count}; unique={unique_tokens}; top={top_token}; checksum={checksum}"
    );

    WorkerLaunchResponse {
        job_id: request.job_id.clone(),
        worker_id,
        status: "completed".to_string(),
        output,
        error: None,
        backend,
        node_id: request.node_id.clone(),
    }
}

fn script_temp_path() -> Result<PathBuf, String> {
    let temp_dir = env::temp_dir();
    let path = temp_dir.join(format!(
        "opengpu-metal-{}.swift",
        uuid::Uuid::new_v4().simple()
    ));
    Ok(path)
}

fn run_metal_request(request: &WorkerLaunchRequest, backend: Backend) -> Option<WorkerLaunchResponse> {
    if backend != Backend::M {
        return None;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        let script_path = match script_temp_path() {
            Ok(path) => path,
            Err(_) => return None,
        };

        if fs::write(&script_path, METAL_RUNNER_SWIFT).is_err() {
            return None;
        }

        let model_name = request.model.clone().unwrap_or_else(|| "default".to_string());
        let output = Command::new("xcrun")
            .arg("swift")
            .arg(&script_path)
            .arg("--job-id")
            .arg(&request.job_id)
            .arg("--node-id")
            .arg(&request.node_id)
            .arg("--prompt")
            .arg(&request.prompt)
            .arg("--model")
            .arg(&model_name)
            .arg("--backend")
            .arg(backend.as_str())
            .output();

        let _ = fs::remove_file(&script_path);

        let output = match output {
            Ok(output) if output.status.success() => output,
            _ => return None,
        };

        let stdout = match String::from_utf8(output.stdout) {
            Ok(text) => text,
            Err(_) => return None,
        };

        serde_json::from_str(stdout.trim()).ok()
    }
}

fn execute_request(request: &WorkerLaunchRequest) -> WorkerLaunchResponse {
    let backend = resolved_backend(request.backend);
    if let Some(response) = run_metal_request(request, backend) {
        return response;
    }

    execute_request_deterministic(request, backend)
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
