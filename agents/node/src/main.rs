mod contracts;
mod http;
mod identity;
mod storage;
mod worker;

use clap::{Parser, Subcommand};
use contracts::{
    AgentRegistration, AgentState, Backend, Heartbeat, JobClaimResponse, JobCompletion, JobRecord,
    NodeCapabilityAdvertisement, WorkerHealthReport, WorkerLaunchRequest, WorkerLaunchResponse,
    WorkerPolicyReport,
};
use http::{signed_get_json, signed_post_json_body};
use identity::{load_identity, DeviceIdentity};
use serde::Serialize;
use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use storage::{
    agent_state_path, config_path, heartbeat_log_path, load_agent_config, load_last_heartbeat,
    save_agent_state, save_heartbeat, AgentConfig,
};

#[derive(Parser, Debug)]
#[command(
    name = "opengpu-agent",
    version,
    about = "MundusX node agent",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Run {
        #[arg(long)]
        once: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        verbose: bool,
        #[arg(long, default_value_t = 5)]
        interval_seconds: u64,
    },
    Register {
        #[arg(long)]
        json: bool,
    },
    Heartbeat {
        #[arg(long)]
        once: bool,
        #[arg(long)]
        json: bool,
    },
    LaunchWorker {
        #[arg(long)]
        job_id: String,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        system_prompt: Option<String>,
        #[arg(long)]
        max_tokens: Option<u32>,
        #[arg(long)]
        temperature: Option<f32>,
        #[arg(long)]
        top_p: Option<f32>,
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    Health {
        #[arg(long)]
        json: bool,
    },
    #[command(hide = true)]
    Worker(worker::WorkerCli),
    Status {
        #[arg(long)]
        json: bool,
    },
    Stop,
}

fn now_unix_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn load_identity_or_exit() -> DeviceIdentity {
    match load_identity() {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            eprintln!("missing identity; run `opengpu start` first");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("failed to load identity: {error}");
            std::process::exit(1);
        }
    }
}

fn load_config_or_exit() -> AgentConfig {
    match load_agent_config() {
        Ok(Some(config)) => config,
        Ok(None) => {
            eprintln!("missing config; run `opengpu start` first");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("failed to load config: {error}");
            std::process::exit(1);
        }
    }
}

fn detect_hostname() -> String {
    if let Ok(value) = std::env::var("HOSTNAME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(value) = std::env::var("COMPUTERNAME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(output) = std::process::Command::new("hostname").output() {
        if output.status.success() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
    }

    "unknown-host".to_string()
}

fn resolved_state(config: &AgentConfig) -> AgentState {
    if !config.connected {
        AgentState::Stopped
    } else if config.paused {
        AgentState::Paused
    } else {
        AgentState::Ready
    }
}

fn worker_readiness(config: &AgentConfig) -> (WorkerHealthReport, WorkerPolicyReport) {
    let model_dir = config.effective_model_dir();
    let health = worker::probe_worker_health(
        &model_dir,
        config.active_model.as_deref(),
        resolved_backend(config),
    );
    let policy = worker::probe_worker_policy(&health, config.contribution_percent);
    (health, policy)
}

fn operational_state(config: &AgentConfig) -> AgentState {
    let (health, policy) = worker_readiness(config);
    let capabilities = build_capabilities(config, &health, policy.allowed);
    if !capabilities.ready_for_jobs {
        AgentState::Paused
    } else {
        resolved_state(config)
    }
}

fn cap_applied_vram_mb(physical_vram_mb: Option<u32>, contribution_percent: u8) -> Option<u32> {
    physical_vram_mb.map(|vram| {
        vram.saturating_mul(contribution_percent as u32)
            .saturating_add(99)
            / 100
    })
}

fn build_capabilities(
    config: &AgentConfig,
    health: &WorkerHealthReport,
    policy_allowed: bool,
) -> NodeCapabilityAdvertisement {
    let backend = resolved_backend(config);
    let active_model = worker::active_model_capability(
        &config.effective_model_dir(),
        config.active_model.as_deref(),
    )
    .or_else(|| {
        config
            .active_model
            .as_ref()
            .map(|name| contracts::ModelCapability {
                name: name.clone(),
                path: health.model_path.clone(),
                format: None,
                quantization: None,
                size_bytes: None,
                estimated_vram_mb: None,
                compatibility: None,
                compatibility_reason: None,
            })
    });

    let mut ready_for_jobs = policy_allowed && health.healthy;
    let mut readiness_reason = None;
    match active_model.as_ref() {
        Some(model) if model.compatibility.as_deref() == Some("rejected") => {
            ready_for_jobs = false;
            readiness_reason = model
                .compatibility_reason
                .clone()
                .or_else(|| Some("active model is not compatible with this node".to_string()));
        }
        Some(_) => {}
        None => {
            ready_for_jobs = false;
            readiness_reason = Some("no active model is configured".to_string());
        }
    }

    if !policy_allowed && readiness_reason.is_none() {
        readiness_reason = Some("worker policy does not allow jobs".to_string());
    } else if !health.healthy && readiness_reason.is_none() {
        readiness_reason = Some("worker health is degraded".to_string());
    }

    NodeCapabilityAdvertisement {
        backend,
        contribution_percent: config.contribution_percent,
        physical_vram_mb: health.cuda_memory_mb,
        usable_vram_mb: cap_applied_vram_mb(health.cuda_memory_mb, config.contribution_percent),
        runtime_mode: health.runtime_mode.clone(),
        active_model,
        ready_for_jobs,
        readiness_reason,
    }
}

fn build_heartbeat_with_state(config: &AgentConfig, agent_state: AgentState) -> Heartbeat {
    let (health, policy) = worker_readiness(config);
    let capabilities = build_capabilities(config, &health, policy.allowed);
    Heartbeat {
        node_id: config.device_id.clone(),
        backend: resolved_backend(config),
        agent_state,
        available_memory_mb: detect_memory_mb(),
        available_gpu_percent: detect_available_gpu_percent(config),
        updated_at: now_unix_seconds(),
        contribution_percent: config.contribution_percent,
        hostname: detect_hostname(),
        identity_trust_path: identity::trust_path(),
        power_source: health.power_source.clone(),
        on_battery: health.on_battery,
        battery_percent: health.battery_percent,
        policy_allowed: policy.allowed,
        policy_reason: policy.reason,
        worker_health: health.clone(),
        capabilities,
    }
}

fn resolved_backend(config: &AgentConfig) -> Backend {
    if config.backend_preference == Backend::Auto {
        #[cfg(target_os = "macos")]
        {
            if std::env::consts::ARCH == "aarch64" {
                return Backend::M;
            }
        }

        if std::env::var_os("NVIDIA_VISIBLE_DEVICES").is_some()
            || std::env::var_os("CUDA_VISIBLE_DEVICES").is_some()
            || worker::probe_cuda_diagnostics().device_available
        {
            return Backend::Cuda;
        }

        Backend::Auto
    } else {
        config.backend_preference
    }
}

fn detect_memory_mb() -> u32 {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output();
        if let Ok(output) = output {
            if output.status.success() {
                if let Ok(text) = String::from_utf8(output.stdout) {
                    if let Ok(bytes) = text.trim().parse::<u64>() {
                        return (bytes / 1024 / 1024) as u32;
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(raw) = fs::read_to_string("/proc/meminfo") {
            for line in raw.lines() {
                if let Some(value) = line.strip_prefix("MemTotal:") {
                    let kb = value
                        .split_whitespace()
                        .next()
                        .and_then(|text| text.parse::<u64>().ok())
                        .unwrap_or(0);
                    return (kb / 1024) as u32;
                }
            }
        }
    }

    0
}

fn detect_available_gpu_percent(config: &AgentConfig) -> u32 {
    100u32.saturating_sub(config.contribution_percent as u32)
}

fn build_registration(config: &AgentConfig, identity: &DeviceIdentity) -> AgentRegistration {
    let (health, policy) = worker_readiness(config);
    AgentRegistration {
        node_id: config.device_id.clone(),
        public_key_fingerprint: identity.fingerprint.clone(),
        public_key_hex: identity.public_key_hex.clone(),
        hostname: detect_hostname(),
        identity_trust_path: identity::trust_path(),
        backend: resolved_backend(config),
        contribution_percent: config.contribution_percent,
        capabilities: build_capabilities(config, &health, policy.allowed),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn build_heartbeat(config: &AgentConfig) -> Heartbeat {
    build_heartbeat_with_state(config, operational_state(config))
}

fn build_worker_launch_request(
    config: &AgentConfig,
    job_id: String,
    prompt: String,
    model: Option<String>,
    system_prompt: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    seed: Option<u64>,
) -> WorkerLaunchRequest {
    WorkerLaunchRequest {
        job_id,
        node_id: config.device_id.clone(),
        backend: resolved_backend(config),
        prompt,
        model,
        system_prompt,
        max_tokens,
        temperature,
        top_p,
        seed,
    }
}

fn emit_json_line<T: Serialize>(value: &T) {
    match serde_json::to_string(value) {
        Ok(output) => {
            println!("{output}");
            let _ = io::stdout().flush();
        }
        Err(error) => {
            eprintln!("failed to serialize output: {error}");
            std::process::exit(1);
        }
    }
}

fn green(text: impl AsRef<str>) -> String {
    format!("\x1b[32m{}\x1b[0m", text.as_ref())
}

fn red(text: impl AsRef<str>) -> String {
    format!("\x1b[31m{}\x1b[0m", text.as_ref())
}

fn is_unknown_node_error(error: &str) -> bool {
    error.to_ascii_lowercase().contains("unknown node")
}

fn send_registration(
    config: &AgentConfig,
    identity: &DeviceIdentity,
    registration: &AgentRegistration,
    verbose: bool,
) -> bool {
    match http::signed_post_json(
        &config.control_plane_url,
        "/v1/register",
        &config.device_id,
        identity,
        registration,
    ) {
        Ok(response) => {
            if verbose {
                println!(
                    "controlPlaneRegister: ok ({})",
                    response.lines().next().unwrap_or("no response line")
                );
            }
            true
        }
        Err(error) => {
            eprintln!("controlPlaneRegister: {error}");
            false
        }
    }
}

fn send_heartbeat(
    config: &AgentConfig,
    identity: &DeviceIdentity,
    heartbeat: &Heartbeat,
    verbose: bool,
) {
    let result = http::signed_post_json(
        &config.control_plane_url,
        "/v1/heartbeat",
        &config.device_id,
        identity,
        heartbeat,
    );
    match result {
        Ok(response) => {
            if verbose {
                println!(
                    "controlPlaneHeartbeat: ok ({})",
                    response.lines().next().unwrap_or("no response line")
                );
            }
        }
        Err(error) => {
            eprintln!("controlPlaneHeartbeat: {error}");
            if is_unknown_node_error(&error) {
                eprintln!("controlPlaneHeartbeat: re-registering missing node");
                let registration = build_registration(config, identity);
                if send_registration(config, identity, &registration, verbose) {
                    match http::signed_post_json(
                        &config.control_plane_url,
                        "/v1/heartbeat",
                        &config.device_id,
                        identity,
                        heartbeat,
                    ) {
                        Ok(response) => {
                            if verbose {
                                println!(
                                    "controlPlaneHeartbeat: ok ({})",
                                    response.lines().next().unwrap_or("no response line")
                                );
                            }
                        }
                        Err(retry_error) => eprintln!("controlPlaneHeartbeat: {retry_error}"),
                    }
                }
            }
        }
    }
}

fn launch_worker_process(
    config: &AgentConfig,
    request: WorkerLaunchRequest,
    json: bool,
) -> Result<contracts::WorkerLaunchResponse, String> {
    let model_dir = config.effective_model_dir();
    let (_, policy) = worker_readiness(config);
    if !policy.allowed {
        return Err(policy
            .reason
            .unwrap_or_else(|| "worker policy denied launch".to_string()));
    }

    match worker::launch_worker(&request, &model_dir) {
        Ok(response) => {
            if json {
                emit_json_line(&response);
            } else {
                println!("workerLaunch: ok");
                println!("workerId: {}", response.worker_id);
                println!("jobId: {}", response.job_id);
                println!("status: {}", response.status);
                println!("backend: {}", response.backend);
                println!("output: {}", response.output);
                if let Some(error) = response.error.as_deref() {
                    println!("error: {error}");
                }
            }
            Ok(response)
        }
        Err(error) => {
            eprintln!("workerLaunch: {error}");
            let _ = save_agent_state(&build_heartbeat(config));
            Err(error)
        }
    }
}

fn print_worker_health(config: &AgentConfig, json: bool) {
    let (health, policy) = worker_readiness(config);

    if json {
        let payload = serde_json::json!({
            "health": health,
            "policy": policy,
        });
        emit_json_line(&payload);
        return;
    }

    println!(
        "workerHealth: {}",
        if health.healthy {
            "healthy"
        } else {
            "degraded"
        }
    );
    println!("modelDir: {}", health.model_dir);
    println!(
        "modelName: {}",
        health.model_name.as_deref().unwrap_or("none")
    );
    println!(
        "modelPath: {}",
        health.model_path.as_deref().unwrap_or("missing")
    );
    println!(
        "llamaCliAvailable: {}",
        if health.llama_cli_available {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "llamaServerAvailable: {}",
        if health.llama_server_available {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "persistentRuntime: {}",
        if health.persistent_runtime_warm {
            "warm"
        } else if health.llama_server_available {
            "unavailable"
        } else {
            "batch"
        }
    );
    println!(
        "persistentRuntimeUrl: {}",
        health.persistent_runtime_url.as_deref().unwrap_or("none")
    );
    println!(
        "blasDeviceAvailable: {}",
        if health.blas_device_available {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "cudaDeviceAvailable: {}",
        if health.cuda_device_available {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "cudaDriverAvailable: {}",
        if health.cuda_driver_available {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "cudaDeviceName: {}",
        health.cuda_device_name.as_deref().unwrap_or("none")
    );
    println!(
        "cudaMemoryMb: {}",
        health
            .cuda_memory_mb
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "cudaLowVramProfile: {}",
        if health.cuda_low_vram_profile {
            "yes"
        } else {
            "no"
        }
    );
    println!("powerSource: {}", health.power_source);
    println!(
        "onBattery: {}",
        if health.on_battery { "yes" } else { "no" }
    );
    println!(
        "batteryPercent: {}",
        health
            .battery_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!("runtimeMode: {}", health.runtime_mode);
    println!("runtimeKind: {}", health.runtime_kind);
    println!("checkedAt: {}", health.checked_at);
    println!(
        "policyAllowed: {}",
        if policy.allowed { "yes" } else { "no" }
    );
    println!(
        "policyRecommendedMaxContributionPercent: {}%",
        policy.recommended_max_contribution_percent
    );
    if let Some(reason) = policy.reason.as_ref() {
        println!("policyReason: {}", reason);
    }
    if !health.notes.is_empty() {
        println!("notes:");
        for note in health.notes {
            println!("  - {}", note);
        }
    }
    if !policy.notes.is_empty() {
        println!("policyNotes:");
        for note in policy.notes {
            println!("  - {}", note);
        }
    }
}

fn claim_next_job(config: &AgentConfig, identity: &DeviceIdentity) -> Option<JobRecord> {
    let path = format!("/v1/jobs/next?node_id={}", config.device_id);
    match signed_get_json::<JobClaimResponse>(
        &config.control_plane_url,
        &path,
        &config.device_id,
        identity,
    ) {
        Ok(response) => response.job,
        Err(error) => {
            eprintln!("controlPlaneClaim: {error}");
            if is_unknown_node_error(&error) {
                eprintln!("controlPlaneClaim: re-registering missing node");
                let registration = build_registration(config, identity);
                if send_registration(config, identity, &registration, false) {
                    match signed_get_json::<JobClaimResponse>(
                        &config.control_plane_url,
                        &path,
                        &config.device_id,
                        identity,
                    ) {
                        Ok(response) => return response.job,
                        Err(retry_error) => eprintln!("controlPlaneClaim: {retry_error}"),
                    }
                }
            }
            None
        }
    }
}

fn job_status_label(status: contracts::JobStatus) -> &'static str {
    match status {
        contracts::JobStatus::Queued => "queued",
        contracts::JobStatus::Assigned => "assigned",
        contracts::JobStatus::Completed => "completed",
        contracts::JobStatus::Failed => "failed",
    }
}

fn control_plane_completion_message(record: &JobRecord, completion: &JobCompletion) -> String {
    format!(
        "controlPlaneComplete: accepted {} {}",
        job_status_label(completion.status),
        record.job_id
    )
}

fn complete_job(config: &AgentConfig, identity: &DeviceIdentity, completion: &JobCompletion) {
    match signed_post_json_body::<_, JobRecord>(
        &config.control_plane_url,
        "/v1/jobs/complete",
        &config.device_id,
        identity,
        completion,
    ) {
        Ok(record) => println!("{}", control_plane_completion_message(&record, completion)),
        Err(error) => eprintln!("controlPlaneComplete: {error}"),
    }
}

fn build_completion_from_worker_response(
    response: WorkerLaunchResponse,
    duration_ms: u64,
) -> JobCompletion {
    let is_completed = response.status == "completed";
    JobCompletion {
        job_id: response.job_id,
        node_id: response.node_id,
        worker_id: response.worker_id,
        backend: response.backend,
        status: if is_completed {
            contracts::JobStatus::Completed
        } else {
            contracts::JobStatus::Failed
        },
        output: if is_completed {
            Some(response.output)
        } else {
            None
        },
        error: if is_completed {
            response.error
        } else {
            response
                .error
                .or_else(|| Some("worker returned failed status".to_string()))
        },
        duration_ms: Some(duration_ms),
        model: response.model,
        runtime_mode: response.runtime_mode,
    }
}

fn build_worker_error_completion(
    job: &JobRecord,
    config: &AgentConfig,
    error: String,
    duration_ms: u64,
) -> JobCompletion {
    JobCompletion {
        job_id: job.job_id.clone(),
        node_id: config.device_id.clone(),
        worker_id: "worker-failed".to_string(),
        backend: resolved_backend(config),
        status: contracts::JobStatus::Failed,
        output: None,
        error: Some(error),
        duration_ms: Some(duration_ms),
        model: job.model.clone().or_else(|| config.active_model.clone()),
        runtime_mode: Some(resolved_backend(config).as_str().to_string()),
    }
}

fn start_busy_heartbeat_supervisor(
    config: AgentConfig,
    identity: DeviceIdentity,
    interval: Duration,
) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::spawn(move || loop {
        match stop_rx.recv_timeout(interval) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let heartbeat = build_heartbeat_with_state(&config, AgentState::Busy);
                let _ = save_agent_state(&heartbeat);
                let _ = save_heartbeat(&heartbeat);
                send_heartbeat(&config, &identity, &heartbeat, false);
            }
        }
    });

    (stop_tx, handle)
}

fn stop_busy_heartbeat_supervisor(stop_tx: mpsc::Sender<()>, handle: thread::JoinHandle<()>) {
    let _ = stop_tx.send(());
    let _ = handle.join();
}

fn process_pending_job(config: &AgentConfig, json: bool, verbose: bool) {
    let identity = load_identity_or_exit();
    let (_, policy) = worker_readiness(config);
    if !policy.allowed {
        if verbose {
            println!(
                "jobPoll: skipped ({})",
                policy.reason.as_deref().unwrap_or("policy denied launch")
            );
        }
        let policy_heartbeat = build_heartbeat_with_state(config, AgentState::Paused);
        let _ = save_agent_state(&policy_heartbeat);
        let _ = save_heartbeat(&policy_heartbeat);
        send_heartbeat(config, &identity, &policy_heartbeat, verbose);
        return;
    }

    if verbose {
        println!("jobPoll: checking control plane");
    }
    let Some(job) = claim_next_job(config, &identity) else {
        if verbose {
            println!("jobPoll: none");
        }
        return;
    };

    println!("jobPoll: claimed {}", job.job_id);

    let busy_heartbeat = build_heartbeat_with_state(config, AgentState::Busy);
    let _ = save_agent_state(&busy_heartbeat);
    let _ = save_heartbeat(&busy_heartbeat);
    send_heartbeat(config, &identity, &busy_heartbeat, verbose);

    let request = WorkerLaunchRequest {
        job_id: job.job_id.clone(),
        node_id: config.device_id.clone(),
        backend: job.backend.unwrap_or_else(|| resolved_backend(config)),
        prompt: job.prompt.clone(),
        model: job.model.clone(),
        system_prompt: job.system_prompt.clone(),
        max_tokens: job.max_tokens,
        temperature: job.temperature,
        top_p: job.top_p,
        seed: job.seed,
    };

    let started_at = Instant::now();
    let (stop_busy_heartbeat, busy_heartbeat_handle) =
        start_busy_heartbeat_supervisor(config.clone(), identity.clone(), Duration::from_secs(5));
    let worker_result = launch_worker_process(config, request, json);
    stop_busy_heartbeat_supervisor(stop_busy_heartbeat, busy_heartbeat_handle);

    match worker_result {
        Ok(response) => {
            let completion = build_completion_from_worker_response(
                response,
                started_at.elapsed().as_millis() as u64,
            );
            complete_job(config, &identity, &completion);
        }
        Err(error) => {
            let completion = build_worker_error_completion(
                &job,
                config,
                error,
                started_at.elapsed().as_millis() as u64,
            );
            complete_job(config, &identity, &completion);
        }
    }

    let ready_heartbeat = build_heartbeat_with_state(config, resolved_state(config));
    let _ = save_agent_state(&ready_heartbeat);
    let _ = save_heartbeat(&ready_heartbeat);
    send_heartbeat(config, &identity, &ready_heartbeat, verbose);
}

fn print_status(json: bool) {
    let config = load_config_or_exit();
    let identity = load_identity_or_exit();
    let state = resolved_state(&config);
    let last_heartbeat = load_last_heartbeat().ok().flatten();

    if json {
        let payload = serde_json::json!({
            "config_path": config_path(),
            "agent_state_path": agent_state_path(),
            "heartbeat_log_path": heartbeat_log_path(),
            "config": config,
            "registration": build_registration(&config, &identity),
            "resolved_state": state,
            "last_heartbeat": last_heartbeat,
        });
        emit_json_line(&payload);
        return;
    }

    println!("configPath: {}", config_path().display());
    println!("agentStatePath: {}", agent_state_path().display());
    println!("heartbeatLogPath: {}", heartbeat_log_path().display());
    println!("nodeId: {}", config.device_id);
    println!("publicKeyFingerprint: {}", identity.fingerprint);
    println!("backend: {}", config.backend_preference);
    println!("state: {}", state);
    println!("connected: {}", if config.connected { "yes" } else { "no" });
    println!("paused: {}", if config.paused { "yes" } else { "no" });
    println!("contributionPercent: {}%", config.contribution_percent);
    println!("controlPlaneUrl: {}", config.control_plane_url);
    println!(
        "lastHeartbeat: {}",
        last_heartbeat
            .as_ref()
            .map(|heartbeat| heartbeat.updated_at.clone())
            .unwrap_or_else(|| "none".to_string())
    );
}

fn should_keep_runtime_warm(config: &AgentConfig) -> bool {
    config.connected && !config.paused
}

fn run_agent(once: bool, json: bool, verbose: bool, interval_seconds: u64) {
    let config = load_config_or_exit();
    let identity = load_identity_or_exit();
    let mut persistent_runtime = if json || !should_keep_runtime_warm(&config) {
        None
    } else {
        match worker::start_persistent_runtime(
            &config.effective_model_dir(),
            config.active_model.as_deref(),
            resolved_backend(&config),
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("persistentRuntime: unavailable ({error}); falling back to batch");
                None
            }
        }
    };
    if let Some(runtime) = persistent_runtime.as_ref() {
        std::env::set_var("OPENGPU_LLAMA_SERVER_URL", runtime.url());
    }
    let registration = build_registration(&config, &identity);
    let heartbeat = build_heartbeat(&config);
    let state = resolved_state(&config);
    let interval = if config.paused {
        interval_seconds.max(30)
    } else {
        interval_seconds.max(5)
    };

    if json {
        let payload = serde_json::json!({
            "registration": registration,
            "heartbeat": heartbeat,
            "state": state,
        });
        emit_json_line(&payload);
        let _ = save_agent_state(&heartbeat);
        let _ = save_heartbeat(&heartbeat);
        return;
    }

    println!("agentVersion: {}", env!("CARGO_PKG_VERSION"));
    println!("nodeId: {}", config.device_id);
    println!("publicKeyFingerprint: {}", identity.fingerprint);
    println!("backend: {}", config.backend_preference);
    println!("state: {}", state);
    println!("intervalSeconds: {}", interval);
    println!("agentStatePath: {}", agent_state_path().display());
    println!("heartbeatLogPath: {}", heartbeat_log_path().display());
    println!("registration: ready");
    println!("heartbeat: ready");
    println!(
        "persistentRuntime: {}",
        persistent_runtime
            .as_ref()
            .map(|runtime| runtime.url())
            .unwrap_or("batch")
    );

    if let Err(error) = save_agent_state(&heartbeat) {
        eprintln!("failed to save agent state: {error}");
        std::process::exit(1);
    }
    if let Err(error) = save_heartbeat(&heartbeat) {
        eprintln!("failed to save heartbeat: {error}");
        std::process::exit(1);
    }

    send_registration(&config, &identity, &registration, verbose);
    send_heartbeat(&config, &identity, &heartbeat, verbose);
    process_pending_job(&config, json, verbose);

    println!("{}", green(format!("connected {}", config.device_id)));
    println!("press Ctrl-C to stop");

    if once {
        return;
    }

    loop {
        thread::sleep(Duration::from_secs(interval));
        let latest_config = match load_agent_config() {
            Ok(Some(config)) => config,
            Ok(None) => {
                eprintln!("agentStop: config missing; cooling persistent runtime");
                drop(persistent_runtime.take());
                std::env::remove_var("OPENGPU_LLAMA_SERVER_URL");
                break;
            }
            Err(error) => {
                eprintln!(
                    "agentStop: failed to reload config ({error}); cooling persistent runtime"
                );
                drop(persistent_runtime.take());
                std::env::remove_var("OPENGPU_LLAMA_SERVER_URL");
                break;
            }
        };
        if !should_keep_runtime_warm(&latest_config) {
            let heartbeat = build_heartbeat(&latest_config);
            let _ = save_agent_state(&heartbeat);
            let _ = save_heartbeat(&heartbeat);
            send_heartbeat(&latest_config, &identity, &heartbeat, verbose);
            drop(persistent_runtime.take());
            std::env::remove_var("OPENGPU_LLAMA_SERVER_URL");
            println!("persistentRuntime: stopped");
            println!(
                "{}",
                red(format!("disconnected {}", latest_config.device_id))
            );
            break;
        }

        let heartbeat = build_heartbeat(&latest_config);
        if let Err(error) = save_agent_state(&heartbeat) {
            eprintln!("failed to save agent state: {error}");
            break;
        }
        if let Err(error) = save_heartbeat(&heartbeat) {
            eprintln!("failed to save heartbeat: {error}");
            break;
        }
        send_heartbeat(&latest_config, &identity, &heartbeat, verbose);
        if verbose {
            println!("heartbeat {} {}", heartbeat.node_id, heartbeat.updated_at);
        }
        process_pending_job(&latest_config, json, verbose);
        let _ = io::stdout().flush();
    }
}

fn print_registration(json: bool) {
    let config = load_config_or_exit();
    let identity = load_identity_or_exit();
    let registration = build_registration(&config, &identity);

    if json {
        emit_json_line(&registration);
        return;
    }

    println!("nodeId: {}", registration.node_id);
    println!(
        "publicKeyFingerprint: {}",
        registration.public_key_fingerprint
    );
    println!("backend: {}", registration.backend);
    println!(
        "contributionPercent: {}%",
        registration.contribution_percent
    );
    println!("agentVersion: {}", registration.agent_version);
}

fn print_heartbeat(once: bool, json: bool) {
    let config = load_config_or_exit();
    let heartbeat = build_heartbeat(&config);

    if json {
        emit_json_line(&heartbeat);
        return;
    }

    println!("nodeId: {}", heartbeat.node_id);
    println!("backend: {}", heartbeat.backend);
    println!("state: {}", heartbeat.agent_state);
    println!("availableMemoryMb: {}", heartbeat.available_memory_mb);
    println!("availableGpuPercent: {}", heartbeat.available_gpu_percent);
    println!("updatedAt: {}", heartbeat.updated_at);
    println!("contributionPercent: {}%", heartbeat.contribution_percent);
    if once {
        println!("mode: once");
    } else {
        println!("mode: live");
    }
}

fn stop_agent() {
    let mut config = load_config_or_exit();
    config.connected = false;
    config.paused = true;
    let heartbeat = build_heartbeat(&config);

    if let Err(error) = save_agent_state(&heartbeat) {
        eprintln!("failed to save stopped state: {error}");
        std::process::exit(1);
    }

    let identity = load_identity_or_exit();
    send_heartbeat(&config, &identity, &heartbeat, false);

    println!("{}", red(format!("disconnected {}", config.device_id)));
    println!("connected: no");
    println!("paused: yes");
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            once,
            json,
            verbose,
            interval_seconds,
        } => run_agent(once, json, verbose, interval_seconds),
        Commands::Register { json } => print_registration(json),
        Commands::Heartbeat { once, json } => print_heartbeat(once, json),
        Commands::LaunchWorker {
            job_id,
            prompt,
            model,
            system_prompt,
            max_tokens,
            temperature,
            top_p,
            seed,
            json,
        } => {
            let config = load_config_or_exit();
            let request = build_worker_launch_request(
                &config,
                job_id,
                prompt,
                model,
                system_prompt,
                max_tokens,
                temperature,
                top_p,
                seed,
            );
            let _ = launch_worker_process(&config, request, json);
        }
        Commands::Health { json } => {
            let config = load_config_or_exit();
            print_worker_health(&config, json);
        }
        Commands::Worker(worker_cli) => worker::worker_main(worker_cli),
        Commands::Status { json } => print_status(json),
        Commands::Stop => stop_agent(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_unknown_node_control_plane_errors() {
        assert!(is_unknown_node_error(
            "controlPlaneHeartbeat: HTTP 401: {\"error\":\"unknown node\"}"
        ));
        assert!(is_unknown_node_error("HTTP 401: UNKNOWN NODE"));
        assert!(!is_unknown_node_error("HTTP 401: invalid signature"));
    }

    fn test_config() -> AgentConfig {
        AgentConfig {
            version: 1,
            device_id: "node-1".to_string(),
            public_key_fingerprint: None,
            profile_name: None,
            auth_token: None,
            connected: true,
            paused: false,
            backend_preference: Backend::Cuda,
            contribution_percent: 25,
            control_plane_url: "http://127.0.0.1:8787".to_string(),
            model_dir: None,
            active_model: Some("tiny-cuda".to_string()),
            models: vec!["tiny-cuda".to_string()],
        }
    }

    #[test]
    fn keeps_persistent_runtime_warm_only_for_active_connected_nodes() {
        let mut config = test_config();
        assert!(should_keep_runtime_warm(&config));

        config.paused = true;
        assert!(!should_keep_runtime_warm(&config));

        config.paused = false;
        config.connected = false;
        assert!(!should_keep_runtime_warm(&config));
    }

    fn test_health(backend: Backend) -> WorkerHealthReport {
        WorkerHealthReport {
            healthy: true,
            model_dir: "/tmp/models".to_string(),
            model_name: Some("tiny-cuda".to_string()),
            model_path: Some("/tmp/models/tiny.gguf".to_string()),
            llama_cli_available: backend != Backend::Cuda,
            llama_server_available: false,
            persistent_runtime_warm: false,
            persistent_runtime_url: None,
            runtime_kind: "batch".to_string(),
            blas_device_available: backend != Backend::Cuda,
            cuda_device_available: backend == Backend::Cuda,
            cuda_driver_available: backend == Backend::Cuda,
            cuda_device_name: if backend == Backend::Cuda {
                Some("NVIDIA GTX".to_string())
            } else {
                None
            },
            cuda_memory_mb: if backend == Backend::Cuda {
                Some(4096)
            } else {
                None
            },
            cuda_low_vram_profile: backend == Backend::Cuda,
            power_source: "ac".to_string(),
            on_battery: false,
            battery_percent: None,
            runtime_mode: backend.as_str().to_string(),
            supported_runtime_modes: vec!["local".to_string()],
            checked_at: "1".to_string(),
            notes: Vec::new(),
        }
    }

    fn write_active_model_manifest(config: &AgentConfig, compatibility: &str) {
        let manifest_dir = config.effective_model_dir().join(".opengpu");
        fs::create_dir_all(&manifest_dir).expect("manifest dir");
        let manifest = serde_json::json!({
            "name": "tiny-cuda",
            "active": true,
            "cached_at": "1",
            "model_dir": config.effective_model_dir(),
            "source_path": "/tmp/models/tiny.gguf",
            "format": "gguf",
            "quantization": "Q4_K_M",
            "size_bytes": 1048576,
            "estimated_vram_mb": 1536,
            "compatibility": compatibility,
            "compatibility_reason": "test compatibility"
        });
        fs::write(manifest_dir.join("tiny-cuda.json"), manifest.to_string()).expect("manifest");
    }

    fn test_job() -> JobRecord {
        JobRecord {
            job_id: "job-1".to_string(),
            request_id: "request-1".to_string(),
            prompt: "summarize".to_string(),
            preferred_backend: Backend::Cuda,
            model: Some("tiny-cuda".to_string()),
            system_prompt: None,
            max_tokens: Some(32),
            temperature: None,
            top_p: None,
            seed: None,
            status: contracts::JobStatus::Assigned,
            submitted_at: "1".to_string(),
            assigned_node_id: Some("node-1".to_string()),
            assigned_at: Some("2".to_string()),
            completed_at: None,
            worker_id: None,
            backend: Some(Backend::Cuda),
            output: None,
            error: None,
        }
    }

    #[test]
    fn completion_preserves_success_runtime_metadata() {
        let response = WorkerLaunchResponse {
            job_id: "job-1".to_string(),
            worker_id: "worker-1".to_string(),
            status: "completed".to_string(),
            output: "answer".to_string(),
            error: None,
            backend: Backend::Cuda,
            node_id: "node-1".to_string(),
            model: Some("tiny-cuda".to_string()),
            runtime_mode: Some("cuda".to_string()),
        };

        let completion = build_completion_from_worker_response(response, 42);

        assert_eq!(completion.status, contracts::JobStatus::Completed);
        assert_eq!(completion.output.as_deref(), Some("answer"));
        assert_eq!(completion.error, None);
        assert_eq!(completion.duration_ms, Some(42));
        assert_eq!(completion.model.as_deref(), Some("tiny-cuda"));
        assert_eq!(completion.runtime_mode.as_deref(), Some("cuda"));
    }

    #[test]
    fn completion_converts_worker_failure_to_actionable_error() {
        let response = WorkerLaunchResponse {
            job_id: "job-1".to_string(),
            worker_id: "worker-1".to_string(),
            status: "failed".to_string(),
            output: "partial".to_string(),
            error: None,
            backend: Backend::Cuda,
            node_id: "node-1".to_string(),
            model: Some("tiny-cuda".to_string()),
            runtime_mode: Some("cuda".to_string()),
        };

        let completion = build_completion_from_worker_response(response, 7);

        assert_eq!(completion.status, contracts::JobStatus::Failed);
        assert_eq!(completion.output, None);
        assert_eq!(
            completion.error.as_deref(),
            Some("worker returned failed status")
        );
        assert_eq!(completion.duration_ms, Some(7));
    }

    #[test]
    fn control_plane_completion_message_includes_reported_status() {
        let job = test_job();
        let completion = JobCompletion {
            job_id: "job-1".to_string(),
            node_id: "node-1".to_string(),
            worker_id: "worker-1".to_string(),
            backend: Backend::Cuda,
            status: contracts::JobStatus::Failed,
            output: None,
            error: Some("runtime failed".to_string()),
            duration_ms: Some(9),
            model: Some("tiny-cuda".to_string()),
            runtime_mode: Some("cuda".to_string()),
        };

        assert_eq!(
            control_plane_completion_message(&job, &completion),
            "controlPlaneComplete: accepted failed job-1"
        );
    }

    #[test]
    fn launch_error_completion_keeps_job_model_and_node_metadata() {
        let config = test_config();
        let job = test_job();

        let completion =
            build_worker_error_completion(&job, &config, "runtime crashed".to_string(), 13);

        assert_eq!(completion.job_id, "job-1");
        assert_eq!(completion.node_id, "node-1");
        assert_eq!(completion.worker_id, "worker-failed");
        assert_eq!(completion.status, contracts::JobStatus::Failed);
        assert_eq!(completion.error.as_deref(), Some("runtime crashed"));
        assert_eq!(completion.duration_ms, Some(13));
        assert_eq!(completion.model.as_deref(), Some("tiny-cuda"));
        assert_eq!(completion.runtime_mode.as_deref(), Some("cuda"));
    }

    #[test]
    fn cuda_capability_advertises_cap_applied_model_budget() {
        let mut config = test_config();
        let temp =
            std::env::temp_dir().join(format!("opengpu-capability-test-{}", now_unix_seconds()));
        config.model_dir = Some(temp.display().to_string());
        config.contribution_percent = 50;
        write_active_model_manifest(&config, "accepted");

        let capability = build_capabilities(&config, &test_health(Backend::Cuda), true);

        assert_eq!(capability.backend, Backend::Cuda);
        assert_eq!(capability.physical_vram_mb, Some(4096));
        assert_eq!(capability.usable_vram_mb, Some(2048));
        assert_eq!(capability.runtime_mode, "cuda");
        assert_eq!(
            capability
                .active_model
                .as_ref()
                .map(|model| model.name.as_str()),
            Some("tiny-cuda")
        );
        assert!(capability.ready_for_jobs);
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn rejected_active_model_is_not_advertised_ready() {
        let mut config = test_config();
        let temp = std::env::temp_dir().join(format!(
            "opengpu-rejected-capability-test-{}",
            now_unix_seconds()
        ));
        config.model_dir = Some(temp.display().to_string());
        write_active_model_manifest(&config, "rejected");

        let capability = build_capabilities(&config, &test_health(Backend::Cuda), true);

        assert!(!capability.ready_for_jobs);
        assert_eq!(
            capability.readiness_reason.as_deref(),
            Some("test compatibility")
        );
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generic_node_without_active_model_is_not_advertised_ready() {
        let mut config = test_config();
        let temp = std::env::temp_dir().join(format!(
            "opengpu-generic-capability-test-{}",
            now_unix_seconds()
        ));
        config.model_dir = Some(temp.display().to_string());
        config.backend_preference = Backend::Auto;
        config.active_model = None;
        config.models = Vec::new();

        let capability = build_capabilities(&config, &test_health(Backend::Auto), true);

        assert!(!capability.ready_for_jobs);
        assert_eq!(
            capability.readiness_reason.as_deref(),
            Some("no active model is configured")
        );
        let _ = fs::remove_dir_all(temp);
    }
}
