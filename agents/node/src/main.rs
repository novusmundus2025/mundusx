mod contracts;
mod http;
mod identity;
mod storage;
mod worker;

use clap::{Parser, Subcommand};
use contracts::{
    AgentRegistration, AgentState, Backend, Heartbeat, JobClaimResponse, JobCompletion, JobRecord,
    WorkerLaunchRequest,
};
use http::{get_json, post_json, post_json_body};
use identity::{load_identity, DeviceIdentity};
use serde::Serialize;
use std::io::{self, Write};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::{
    agent_state_path, config_path, heartbeat_log_path, load_agent_config, load_last_heartbeat,
    save_agent_state, save_heartbeat, AgentConfig,
};

#[derive(Parser, Debug)]
#[command(
    name = "opengpu-agent",
    version,
    about = "OpenGPU node agent",
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

fn resolved_state(config: &AgentConfig) -> AgentState {
    if !config.connected {
        AgentState::Stopped
    } else if config.paused {
        AgentState::Paused
    } else {
        AgentState::Ready
    }
}

fn build_heartbeat_with_state(config: &AgentConfig, agent_state: AgentState) -> Heartbeat {
    Heartbeat {
        node_id: config.device_id.clone(),
        backend: resolved_backend(config),
        agent_state,
        available_memory_mb: detect_memory_mb(),
        available_gpu_percent: detect_available_gpu_percent(config),
        updated_at: now_unix_seconds(),
        contribution_percent: config.contribution_percent,
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
    AgentRegistration {
        node_id: config.device_id.clone(),
        public_key_fingerprint: identity.fingerprint.clone(),
        backend: config.backend_preference,
        contribution_percent: config.contribution_percent,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn build_heartbeat(config: &AgentConfig) -> Heartbeat {
    build_heartbeat_with_state(config, resolved_state(config))
}

fn build_worker_launch_request(
    config: &AgentConfig,
    job_id: String,
    prompt: String,
    model: Option<String>,
) -> WorkerLaunchRequest {
    WorkerLaunchRequest {
        job_id,
        node_id: config.device_id.clone(),
        backend: resolved_backend(config),
        prompt,
        model,
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

fn send_registration(config: &AgentConfig, registration: &AgentRegistration) {
    match post_json(&config.control_plane_url, "/v1/register", registration) {
        Ok(response) => println!("controlPlaneRegister: ok ({})", response.lines().next().unwrap_or("no response line")),
        Err(error) => eprintln!("controlPlaneRegister: {error}"),
    }
}

fn send_heartbeat(config: &AgentConfig, heartbeat: &Heartbeat) {
    match post_json(&config.control_plane_url, "/v1/heartbeat", heartbeat) {
        Ok(response) => println!("controlPlaneHeartbeat: ok ({})", response.lines().next().unwrap_or("no response line")),
        Err(error) => eprintln!("controlPlaneHeartbeat: {error}"),
    }
}

fn launch_worker_process(config: &AgentConfig, request: WorkerLaunchRequest, json: bool) -> Result<contracts::WorkerLaunchResponse, String> {
    let model_dir = config.effective_model_dir();
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

fn claim_next_job(config: &AgentConfig) -> Option<JobRecord> {
    let path = format!("/v1/jobs/next?node_id={}", config.device_id);
    match get_json::<JobClaimResponse>(&config.control_plane_url, &path) {
        Ok(response) => response.job,
        Err(error) => {
            eprintln!("controlPlaneClaim: {error}");
            None
        }
    }
}

fn complete_job(config: &AgentConfig, completion: &JobCompletion) {
    match post_json_body::<_, JobRecord>(&config.control_plane_url, "/v1/jobs/complete", completion) {
        Ok(record) => println!("controlPlaneComplete: ok {}", record.job_id),
        Err(error) => eprintln!("controlPlaneComplete: {error}"),
    }
}

fn process_pending_job(config: &AgentConfig, json: bool) {
    println!("jobPoll: checking control plane");
    let Some(job) = claim_next_job(config) else {
        println!("jobPoll: none");
        return;
    };

    println!("jobPoll: claimed {}", job.job_id);

    let busy_heartbeat = build_heartbeat_with_state(config, AgentState::Busy);
    let _ = save_agent_state(&busy_heartbeat);
    let _ = save_heartbeat(&busy_heartbeat);
    send_heartbeat(config, &busy_heartbeat);

    let request = WorkerLaunchRequest {
        job_id: job.job_id.clone(),
        node_id: config.device_id.clone(),
        backend: job.backend.unwrap_or_else(|| resolved_backend(config)),
        prompt: job.prompt.clone(),
        model: job.model.clone(),
    };

    match launch_worker_process(config, request, json) {
        Ok(response) => {
            let is_completed = response.status == "completed";
            let completion = JobCompletion {
                job_id: response.job_id.clone(),
                node_id: response.node_id.clone(),
                worker_id: response.worker_id.clone(),
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
            };
            complete_job(config, &completion);
        }
        Err(error) => {
            let completion = JobCompletion {
                job_id: job.job_id.clone(),
                node_id: config.device_id.clone(),
                worker_id: "worker-failed".to_string(),
                backend: resolved_backend(config),
                status: contracts::JobStatus::Failed,
                output: None,
                error: Some(error),
            };
            complete_job(config, &completion);
        }
    }

    let ready_heartbeat = build_heartbeat_with_state(config, resolved_state(config));
    let _ = save_agent_state(&ready_heartbeat);
    let _ = save_heartbeat(&ready_heartbeat);
    send_heartbeat(config, &ready_heartbeat);
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

fn run_agent(once: bool, json: bool, interval_seconds: u64) {
    let config = load_config_or_exit();
    let identity = load_identity_or_exit();
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

    if let Err(error) = save_agent_state(&heartbeat) {
        eprintln!("failed to save agent state: {error}");
        std::process::exit(1);
    }
    if let Err(error) = save_heartbeat(&heartbeat) {
        eprintln!("failed to save heartbeat: {error}");
        std::process::exit(1);
    }

    send_registration(&config, &registration);
    send_heartbeat(&config, &heartbeat);
    process_pending_job(&config, json);

    println!("agent ready");
    println!("press Ctrl-C to stop");

    if once {
        return;
    }

    loop {
        thread::sleep(Duration::from_secs(interval));
        let heartbeat = build_heartbeat(&config);
        if let Err(error) = save_agent_state(&heartbeat) {
            eprintln!("failed to save agent state: {error}");
            break;
        }
        if let Err(error) = save_heartbeat(&heartbeat) {
            eprintln!("failed to save heartbeat: {error}");
            break;
        }
        send_heartbeat(&config, &heartbeat);
        println!("heartbeat {} {}", heartbeat.node_id, heartbeat.updated_at);
        process_pending_job(&config, json);
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
    println!("publicKeyFingerprint: {}", registration.public_key_fingerprint);
    println!("backend: {}", registration.backend);
    println!("contributionPercent: {}%", registration.contribution_percent);
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

    send_heartbeat(&config, &heartbeat);

    println!("stopped agent for {}", config.device_id);
    println!("connected: no");
    println!("paused: yes");
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run {
            once,
            json,
            interval_seconds,
        } => run_agent(once, json, interval_seconds),
        Commands::Register { json } => print_registration(json),
        Commands::Heartbeat { once, json } => print_heartbeat(once, json),
        Commands::LaunchWorker {
            job_id,
            prompt,
            model,
            json,
        } => {
            let config = load_config_or_exit();
            let request = build_worker_launch_request(&config, job_id, prompt, model);
            let _ = launch_worker_process(&config, request, json);
        }
        Commands::Worker(worker_cli) => worker::worker_main(worker_cli),
        Commands::Status { json } => print_status(json),
        Commands::Stop => stop_agent(),
    }
}
