mod contracts;
pub mod harness;
pub mod harness_tools;
mod http;
mod identity;
mod local_api;
mod storage;
mod worker;

use clap::{Parser, Subcommand};
use contracts::{
    AgentRegistration, AgentState, Backend, Heartbeat, JobClaimResponse, JobCompletion, JobRecord,
    JobStreamAck, JobStreamDelta, NodeAdmissionStatus, NodeCapabilityAdvertisement,
    NodeCapabilityProfile, NodeRole, WorkerHealthReport, WorkerLaunchRequest, WorkerLaunchResponse,
    WorkerPolicyReport,
};
use http::{signed_get_json, signed_post_json_body};
use identity::{load_identity, DeviceIdentity};
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use storage::{
    agent_state_path, config_path, heartbeat_log_path, load_agent_config, load_last_heartbeat,
    save_agent_config, save_agent_state, save_heartbeat, AgentConfig,
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
        mode: Option<String>,
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
    let mut health = worker::probe_worker_health(
        &model_dir,
        config.active_model.as_deref(),
        resolved_backend(config),
        config.contributed_cluster.as_ref(),
    );
    // The contributor's own limit governs every runtime. Derived figures —
    // memory ladders for a managed runtime, reported ceilings for a contributed
    // cluster — only apply when the contributor has not said otherwise.
    health.parallel_slots = if let Some(max_jobs) = config.max_jobs.filter(|value| *value > 0) {
        max_jobs.min(u8::MAX as u32) as u8
    } else if let Some(cluster) = config.contributed_cluster.as_ref() {
        // The runtime publishes how many concurrent sequences it can hold, so
        // use that instead of the old hardcoded 1. Fall back to one slot only
        // when it reports nothing.
        contributed_cluster_slots(cluster)
    } else {
        worker::recommended_parallel_slots(
            resolved_backend(config),
            health.cuda_memory_mb,
            Some(detect_memory_mb()),
            config.contribution_percent,
            config.active_model.as_deref(),
        )
    };
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

/// Safety ceiling used only when the runtime reports theoretical KV capacity
/// but does not expose its configured `max_num_seqs` scheduler limit.
const MAX_UNVERIFIED_CLUSTER_SLOTS: u32 = 16;

/// Concurrent jobs to advertise for a contributed cluster.
fn contributed_cluster_slots(cluster: &crate::storage::ContributedCluster) -> u8 {
    let reported = match cluster.max_num_seqs {
        Some(configured) => configured.min(cluster.max_concurrency.unwrap_or(configured)),
        None => cluster
            .max_concurrency
            .unwrap_or(1)
            .min(MAX_UNVERIFIED_CLUSTER_SLOTS),
    };
    reported.max(1).min(u8::MAX as u32) as u8
}

/// Usable memory for a node serving a contributed cluster.
///
/// Available memory is the wrong basis: it is low precisely *because* the
/// cluster has loaded a large model, so a busy 121 GB machine classified itself
/// as `micro`. The runtime reports the share of the machine it took
/// (`gpu_memory_utilization`), and that share is what this node contributes.
fn contributed_cluster_usable_memory_mb(
    physical_memory_mb: u32,
    cluster: &crate::storage::ContributedCluster,
) -> u32 {
    const DEFAULT_UTILIZATION: f32 = 0.85;
    let utilization = cluster
        .memory_utilization
        .filter(|value| *value > 0.0 && *value <= 1.0)
        .unwrap_or(DEFAULT_UTILIZATION);
    ((physical_memory_mb as f32) * utilization) as u32
}

fn cap_applied_memory_mb(physical_memory_mb: u32, contribution_percent: u8) -> u32 {
    physical_memory_mb
        .saturating_mul(contribution_percent as u32)
        .saturating_add(99)
        / 100
}

/// Ordinal for the capacity ladder, so role thresholds can be expressed as
/// "this class or above". Unknown names rank lowest.
fn capacity_rank(class: &str) -> u8 {
    match class.trim().to_ascii_lowercase().as_str() {
        "server" => 6,
        "synthesis" => 5,
        "heavy" => 4,
        "performance" => 3,
        "standard" => 2,
        "micro" => 1,
        _ => 0,
    }
}

fn classify_capacity(
    backend: Backend,
    usable_memory_mb: u32,
    usable_vram_mb: Option<u32>,
) -> &'static str {
    let vram = usable_vram_mb.unwrap_or(0);
    if backend == Backend::Vllm {
        "server"
    } else if usable_memory_mb >= 65_536 || vram >= 49_152 {
        "synthesis"
    } else if usable_memory_mb >= 32_768 || vram >= 24_576 {
        "heavy"
    } else if usable_memory_mb >= 16_384 || vram >= 12_288 {
        "performance"
    } else if usable_memory_mb >= 8_192 || vram >= 6_144 {
        "standard"
    } else {
        "micro"
    }
}

fn build_capabilities(
    config: &AgentConfig,
    health: &WorkerHealthReport,
    policy_allowed: bool,
) -> NodeCapabilityAdvertisement {
    let backend = resolved_backend(config);
    let physical_memory_mb = detect_memory_mb();
    let available_memory_mb = detect_available_memory_mb();
    let usable_memory_mb = match config.contributed_cluster.as_ref() {
        // A contributed cluster is sized from the machine and the share the
        // runtime took, not from whatever memory happens to be free right now.
        Some(cluster) => contributed_cluster_usable_memory_mb(physical_memory_mb, cluster),
        None => cap_applied_memory_mb(physical_memory_mb, config.contribution_percent)
            .min(available_memory_mb),
    };
    let cluster = config.contributed_cluster.as_ref();
    // A contributed cluster owns its own memory, so the contribution cap does not
    // translate into a VRAM budget for it.
    let usable_vram_mb = match cluster {
        Some(_) => None,
        None => cap_applied_vram_mb(health.cuda_memory_mb, config.contribution_percent),
    };
    let active_model = match cluster {
        Some(cluster) => cluster
            .model
            .clone()
            .or_else(|| cluster.models.first().cloned())
            .map(|name| contracts::ModelCapability {
                name,
                path: None,
                format: None,
                quantization: None,
                size_bytes: cluster.model_bytes,
                estimated_vram_mb: None,
                compatibility: None,
                compatibility_reason: None,
                active: true,
                warm: true,
                ..contracts::ModelCapability::default()
            }),
        None => worker::active_model_capability(
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
                    active: true,
                    ..contracts::ModelCapability::default()
                })
        }),
    };

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
            readiness_reason = Some(if cluster.is_some() {
                "contributed cluster advertises no model".to_string()
            } else {
                "no active model is configured".to_string()
            });
        }
    }

    if !policy_allowed && readiness_reason.is_none() {
        readiness_reason = Some("worker policy does not allow jobs".to_string());
    } else if !health.healthy && readiness_reason.is_none() {
        readiness_reason = Some("worker health is degraded".to_string());
    }

    NodeCapabilityAdvertisement {
        schema_version: 4,
        backend,
        contribution_percent: config.contribution_percent,
        physical_memory_mb: Some(physical_memory_mb),
        usable_memory_mb: Some(usable_memory_mb),
        available_memory_mb: Some(available_memory_mb),
        physical_vram_mb: health.cuda_memory_mb,
        usable_vram_mb,
        runtime_mode: health.runtime_mode.clone(),
        parallel_slots: health.parallel_slots,
        capacity_class: cluster
            .map(|cluster| {
                let configured = cluster.capacity_class.trim().to_ascii_lowercase();
                if capacity_rank(&configured) == 0 {
                    "server".to_string()
                } else {
                    configured
                }
            })
            .unwrap_or_else(|| {
                classify_capacity(backend, usable_memory_mb, usable_vram_mb).to_string()
            }),
        supported_roles: Vec::new(),
        supported_tools: Vec::new(),
        active_model,
        ready_for_jobs,
        readiness_reason,
    }
}

fn default_context_tokens_for_model(model: &contracts::ModelCapability) -> u32 {
    let name = model.name.to_ascii_lowercase();
    if name.contains("32b") || name.contains("14b") || name.contains("coder") {
        16_384
    } else if name.contains("7b") || name.contains("8b") || name.contains("3b") {
        8_192
    } else {
        4_096
    }
}

fn model_capacity_class(model: &contracts::ModelCapability, node_capacity_class: &str) -> String {
    let name = model.name.to_ascii_lowercase();
    let inferred = if name.contains("70b") || name.contains("72b") || name.contains("405b") {
        "synthesis"
    } else if name.contains("30b") || name.contains("32b") || name.contains("34b") {
        "heavy"
    } else if name.contains("13b") || name.contains("14b") {
        "performance"
    } else if name.contains("7b") || name.contains("8b") {
        "standard"
    } else if name.contains("3b")
        || name.contains("1b")
        || name.contains("0.5b")
        || name.contains("tiny")
        || name.contains("mini")
    {
        "micro"
    } else {
        node_capacity_class
    };
    if capacity_rank(inferred) > capacity_rank(node_capacity_class) {
        node_capacity_class.to_string()
    } else {
        inferred.to_string()
    }
}

fn enrich_model_capability(
    mut model: contracts::ModelCapability,
    node_capacity_class: &str,
    cluster_capabilities: &[String],
    cluster_context_tokens: Option<u32>,
) -> contracts::ModelCapability {
    let name = model.name.to_ascii_lowercase();
    model.capacity_class = model_capacity_class(&model, node_capacity_class);
    model.context_tokens = cluster_context_tokens
        .or(model.context_tokens)
        .or(Some(default_context_tokens_for_model(&model)));
    if model.output_capacity_mode.as_deref() == Some("context_window") {
        model.max_output_tokens = None;
    } else {
        model.max_output_tokens = model.max_output_tokens.or_else(|| {
            let capacity_limit = match model.capacity_class.as_str() {
                "synthesis" | "server" => 16_384,
                "heavy" => 8_192,
                "performance" => 4_096,
                _ => 2_048,
            };
            Some(
                model
                    .context_tokens
                    .map_or(capacity_limit, |context| capacity_limit.min(context)),
            )
        });
    }

    let declared = |needle: &str| {
        cluster_capabilities
            .iter()
            .chain(model.specialties.iter())
            .any(|value| value.to_ascii_lowercase().contains(needle))
    };
    let rank = capacity_rank(&model.capacity_class);
    let coding = name.contains("code") || name.contains("coder") || declared("code");
    let mut tasks = vec!["chat".to_string(), "math".to_string()];
    let mut roles = vec![NodeRole::Chat, NodeRole::Batch, NodeRole::ChunkAnalysis];
    if rank >= capacity_rank("standard") {
        tasks.push("reasoning".to_string());
    }
    if !name.contains("embed") || coding {
        tasks.push("small_coding".to_string());
        roles.push(NodeRole::Coding);
    }
    if rank >= capacity_rank("performance") {
        tasks.extend(["medium_coding".to_string(), "research".to_string()]);
        roles.push(NodeRole::Reducer);
    }
    if rank >= capacity_rank("heavy") {
        tasks.extend(["large_coding".to_string(), "synthesizer".to_string()]);
        roles.push(NodeRole::Synthesizer);
    }
    if declared("tool") {
        tasks.push("tool_use".to_string());
        roles.push(NodeRole::ToolUse);
    }
    if declared("vision") {
        tasks.push("vision".to_string());
        roles.push(NodeRole::Vision);
    }
    if declared("embed") || name.contains("embed") {
        tasks.push("embedding".to_string());
        roles.push(NodeRole::Embedding);
    }
    if model.supports_structured_output || declared("structured") || declared("json") {
        tasks.push("structured_output".to_string());
        model.supports_structured_output = true;
    }
    tasks.sort();
    tasks.dedup();
    roles.sort_by_key(|role| role.as_str());
    roles.dedup();
    model.task_capabilities = tasks;
    model.roles = roles;
    model
}

fn node_roles_for(
    backend: Backend,
    health: &WorkerHealthReport,
    capabilities: &NodeCapabilityAdvertisement,
    available_memory_mb: u32,
) -> Vec<NodeRole> {
    if !capabilities.ready_for_jobs {
        return Vec::new();
    }

    // A contributed cluster has no MundusX-managed VRAM budget and does its own
    // batching, so the VRAM and slot thresholds below can never fire for it.
    // Its roles come from the capacity class already derived from model size.
    let cluster_class = if health.runtime_mode == "contributed-cluster" {
        Some(capabilities.capacity_class.as_str())
    } else {
        None
    };
    let cluster_rank = cluster_class.map(capacity_rank).unwrap_or(0);

    let mut roles = vec![NodeRole::Chat, NodeRole::Batch, NodeRole::ChunkAnalysis];
    if health.runtime_mode == "vllm"
        || cluster_rank >= capacity_rank("standard")
        || matches!(backend, Backend::Cuda | Backend::Vulkan | Backend::M)
    {
        roles.push(NodeRole::Coding);
    }
    let usable_vram = capabilities.usable_vram_mb.unwrap_or(0);
    if health.runtime_mode == "vllm"
        || cluster_rank >= capacity_rank("heavy")
        || usable_vram >= 8_192
        || health.parallel_slots >= 2
        || (backend == Backend::M && available_memory_mb >= 65_536)
    {
        roles.push(NodeRole::Reducer);
    }
    if health.runtime_mode == "vllm"
        || cluster_rank >= capacity_rank("synthesis")
        || usable_vram >= 12_288
        || (backend == Backend::M && available_memory_mb >= 32_768)
        || available_memory_mb >= 65_536
    {
        roles.push(NodeRole::Synthesizer);
    }
    roles.sort_by_key(|role| role.as_str());
    roles.dedup();
    roles
}

fn build_scheduler_capabilities(
    config: &AgentConfig,
    health: &WorkerHealthReport,
    capabilities: &NodeCapabilityAdvertisement,
    available_memory_mb: u32,
    available_gpu_percent: u32,
) -> NodeCapabilityProfile {
    let backend = resolved_backend(config);
    let model = capabilities.active_model.clone();
    let cluster = config.contributed_cluster.as_ref();
    let cluster_capabilities = cluster
        .map(|entry| entry.model_capabilities.as_slice())
        .unwrap_or_default();
    let mut models = if let Some(cluster) = cluster {
        let mut names = cluster.models.clone();
        if let Some(name) = cluster.model.clone() {
            names.push(name);
        }
        names.sort();
        names.dedup();
        names
            .into_iter()
            .map(|name| contracts::ModelCapability {
                active: cluster.model.as_deref() == Some(name.as_str()),
                warm: health.persistent_runtime_warm
                    && cluster.model.as_deref() == Some(name.as_str()),
                name,
                size_bytes: cluster.model_bytes,
                // Contributed runtimes generally share one context budget
                // between prompt and completion. Advertise that contract
                // explicitly instead of inventing a fixed output ceiling.
                max_output_tokens: None,
                output_capacity_mode: Some("context_window".to_string()),
                ..contracts::ModelCapability::default()
            })
            .collect::<Vec<_>>()
    } else {
        let usable_vram = capabilities.usable_vram_mb.map(u64::from);
        let mut installed = worker::available_model_capabilities(&config.effective_model_dir())
            .into_iter()
            .filter(|entry| {
                usable_vram.is_none_or(|budget| {
                    entry
                        .estimated_vram_mb
                        .is_none_or(|required| required <= budget)
                })
            })
            .collect::<Vec<_>>();
        if installed.is_empty() {
            installed.extend(model.clone());
        }
        installed
    };
    for entry in &mut models {
        entry.warm |= entry.active && health.persistent_runtime_warm;
        *entry = enrich_model_capability(
            entry.clone(),
            &capabilities.capacity_class,
            cluster_capabilities,
            cluster.and_then(|entry| entry.model_context_tokens),
        );
    }
    let context_tokens = models.iter().filter_map(|entry| entry.context_tokens).max();
    let available_vram_mb = capabilities
        .usable_vram_mb
        .or(capabilities.physical_vram_mb)
        .or(health.cuda_memory_mb);
    let total_vram_mb = capabilities.physical_vram_mb.or(health.cuda_memory_mb);
    let current_load_percent = Some(100_u8.saturating_sub(available_gpu_percent.min(100) as u8));
    // A contributed cluster node has no local active model, so the name-derived
    // flags must come from the model the cluster advertises.
    let active_model_name = model
        .as_ref()
        .map(|entry| entry.name.clone())
        .or_else(|| config.active_model.clone())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // The runtime tells us what its model can do, so prefer that over guesswork.
    let advertised_capabilities: Vec<String> = cluster
        .map(|cluster| {
            cluster
                .model_capabilities
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let advertises = |needle: &str| {
        advertised_capabilities
            .iter()
            .any(|value| value.contains(needle))
    };
    let mut roles = if capabilities.ready_for_jobs {
        models
            .iter()
            .flat_map(|entry| entry.roles.iter().copied())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if roles.is_empty() && capabilities.ready_for_jobs {
        roles = node_roles_for(backend, health, capabilities, available_memory_mb);
    }
    roles.sort_by_key(|role| role.as_str());
    roles.dedup();
    // Two signals: capabilities the listing advertises, and — for vLLM, which
    // publishes no capability array — a loaded tool-call parser.
    let supports_tools = advertises("tool")
        || cluster
            .map(|cluster| cluster.supports_tool_calls)
            .unwrap_or(false);
    let mut supported_tools = capabilities.supported_tools.clone();
    if supports_tools {
        supported_tools.push("tool_use".to_string());
    }
    if roles.contains(&NodeRole::Coding) {
        supported_tools.push("repository".to_string());
    }
    supported_tools.sort();
    supported_tools.dedup();

    NodeCapabilityProfile {
        schema_version: capabilities.schema_version,
        models,
        physical_memory_mb: capabilities.physical_memory_mb,
        usable_memory_mb: capabilities.usable_memory_mb,
        available_memory_mb: capabilities.available_memory_mb,
        capacity_class: capabilities.capacity_class.clone(),
        max_context_tokens: context_tokens,
        max_num_seqs: cluster.and_then(|entry| entry.max_num_seqs),
        kv_cache_size_tokens: cluster.and_then(|entry| entry.kv_cache_tokens),
        total_vram_mb,
        available_vram_mb,
        supports_vision: advertises("vision"),
        supports_embeddings: advertises("embed") || active_model_name.contains("embed"),
        supports_tools,
        max_parallel_jobs: health.parallel_slots.max(1) as u32,
        current_load_percent,
        roles,
        skill_tags: vec![
            format!("backend:{}", backend.as_str()),
            format!("runtime:{}", health.runtime_mode),
        ],
        supported_tools,
    }
}

fn build_heartbeat_with_state(config: &AgentConfig, agent_state: AgentState) -> Heartbeat {
    let (mut health, policy) = worker_readiness(config);
    let mut capabilities = build_capabilities(config, &health, policy.allowed);
    let available_memory_mb = capabilities
        .available_memory_mb
        .unwrap_or_else(detect_available_memory_mb);
    let available_gpu_percent = detect_available_gpu_percent(config);
    health.capabilities = build_scheduler_capabilities(
        config,
        &health,
        &capabilities,
        available_memory_mb,
        available_gpu_percent,
    );
    capabilities.supported_roles = health.capabilities.roles.clone();
    capabilities.supported_tools = supported_tools_for(&health.capabilities);
    Heartbeat {
        node_id: config.device_id.clone(),
        backend: resolved_backend(config),
        agent_state,
        available_memory_mb,
        available_gpu_percent,
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

        #[cfg(target_os = "windows")]
        if worker::probe_vulkan_device().is_ok() {
            return Backend::Vulkan;
        }

        Backend::Auto
    } else {
        config.backend_preference
    }
}

fn detect_memory_mb() -> u32 {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            dwMemoryLoad: 0,
            ullTotalPhys: 0,
            ullAvailPhys: 0,
            ullTotalPageFile: 0,
            ullAvailPageFile: 0,
            ullTotalVirtual: 0,
            ullAvailVirtual: 0,
            ullAvailExtendedVirtual: 0,
        };

        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok != 0 {
            return (status.ullTotalPhys / 1024 / 1024) as u32;
        }
    }

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

fn detect_available_memory_mb() -> u32 {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            dwMemoryLoad: 0,
            ullTotalPhys: 0,
            ullAvailPhys: 0,
            ullTotalPageFile: 0,
            ullAvailPageFile: 0,
            ullTotalVirtual: 0,
            ullAvailVirtual: 0,
            ullAvailExtendedVirtual: 0,
        };
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            return (status.ullAvailPhys / 1024 / 1024) as u32;
        }
    }

    #[cfg(target_os = "linux")]
    if let Ok(raw) = fs::read_to_string("/proc/meminfo") {
        if let Some(kb) = raw.lines().find_map(|line| {
            line.strip_prefix("MemAvailable:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        }) {
            return (kb / 1024) as u32;
        }
    }

    detect_memory_mb()
}

fn supported_tools_for(capabilities: &NodeCapabilityProfile) -> Vec<String> {
    let mut tools = Vec::new();
    if capabilities.supports_tools {
        tools.push("tool_use".to_string());
    }
    if capabilities.roles.contains(&NodeRole::Coding) {
        tools.push("repository".to_string());
    }
    tools.sort();
    tools.dedup();
    tools
}

fn detect_available_gpu_percent(config: &AgentConfig) -> u32 {
    100u32.saturating_sub(config.contribution_percent as u32)
}

fn build_registration(config: &AgentConfig, identity: &DeviceIdentity) -> AgentRegistration {
    let (mut health, policy) = worker_readiness(config);
    let mut capabilities = build_capabilities(config, &health, policy.allowed);
    let available_memory_mb = capabilities
        .available_memory_mb
        .unwrap_or_else(detect_available_memory_mb);
    health.capabilities = build_scheduler_capabilities(
        config,
        &health,
        &capabilities,
        available_memory_mb,
        detect_available_gpu_percent(config),
    );
    capabilities.supported_roles = health.capabilities.roles.clone();
    capabilities.supported_tools = supported_tools_for(&health.capabilities);
    AgentRegistration {
        node_id: config.device_id.clone(),
        public_key_fingerprint: identity.fingerprint.clone(),
        public_key_hex: identity.public_key_hex.clone(),
        hostname: detect_hostname(),
        identity_trust_path: identity::trust_path(),
        backend: resolved_backend(config),
        contribution_percent: config.contribution_percent,
        capability_fabric_version: "1.0".to_string(),
        capabilities,
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn build_heartbeat(config: &AgentConfig) -> Heartbeat {
    build_heartbeat_with_state(config, operational_state(config))
}

fn effective_ready_for_jobs(
    capabilities: &NodeCapabilityAdvertisement,
    control_plane_status: Option<&NodeAdmissionStatus>,
) -> bool {
    capabilities.ready_for_jobs
        && control_plane_status
            .map(|status| status.policy_allowed)
            .unwrap_or(true)
}

fn effective_readiness_reason(
    capabilities: &NodeCapabilityAdvertisement,
    control_plane_status: Option<&NodeAdmissionStatus>,
) -> Option<String> {
    if let Some(status) = control_plane_status {
        if !status.policy_allowed {
            return status
                .policy_reason
                .clone()
                .or_else(|| status.computed_policy_reason.clone())
                .or_else(|| Some("control-plane admission policy blocked this node".to_string()));
        }
    }

    capabilities.readiness_reason.clone()
}

fn print_capability_summary_with_control_plane(
    capabilities: &NodeCapabilityAdvertisement,
    control_plane_status: Option<&NodeAdmissionStatus>,
) {
    println!(
        "readyForJobs: {}",
        if effective_ready_for_jobs(capabilities, control_plane_status) {
            "yes"
        } else {
            "no"
        }
    );
    if let Some(reason) = effective_readiness_reason(capabilities, control_plane_status) {
        println!("readinessReason: {reason}");
    }
    if let Some(status) = control_plane_status {
        println!(
            "controlPlanePolicyAllowed: {}",
            if status.policy_allowed { "yes" } else { "no" }
        );
        println!("controlPlaneState: {}", status.state);
    }
    println!("runtimeMode: {}", capabilities.runtime_mode);
    println!("capabilitySchemaVersion: {}", capabilities.schema_version);
    println!("capacityClass: {}", capabilities.capacity_class);
    println!("parallelSlots: {}", capabilities.parallel_slots);
    if let Some(memory_mb) = capabilities.physical_memory_mb {
        println!("physicalMemoryMb: {memory_mb}");
    }
    if let Some(memory_mb) = capabilities.usable_memory_mb {
        println!("usableMemoryMb: {memory_mb}");
    }
    if let Some(memory_mb) = capabilities.available_memory_mb {
        println!("availableMemoryMb: {memory_mb}");
    }
    if let Some(vram_mb) = capabilities.physical_vram_mb {
        println!("physicalVramMb: {vram_mb}");
    }
    if let Some(vram_mb) = capabilities.usable_vram_mb {
        println!("usableVramMb: {vram_mb}");
    }
    if let Some(model) = capabilities.active_model.as_ref() {
        println!("activeModel: {}", model.name);
        if let Some(compatibility) = model.compatibility.as_ref() {
            println!("activeModelCompatibility: {compatibility}");
        }
        if let Some(reason) = model.compatibility_reason.as_ref() {
            println!("activeModelCompatibilityReason: {reason}");
        }
    } else {
        println!("activeModel: unset");
    }
}

fn print_capability_summary(capabilities: &NodeCapabilityAdvertisement) {
    print_capability_summary_with_control_plane(capabilities, None);
}

fn control_plane_blocks_jobs(status: Option<&NodeAdmissionStatus>) -> Option<String> {
    let status = status?;
    if status.policy_allowed {
        return None;
    }

    Some(
        status
            .policy_reason
            .clone()
            .or_else(|| status.computed_policy_reason.clone())
            .unwrap_or_else(|| "control-plane admission policy blocked this node".to_string()),
    )
}

fn build_worker_launch_request(
    config: &AgentConfig,
    job_id: String,
    prompt: String,
    model: Option<String>,
    mode: Option<String>,
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
        stream: false,
        prompt,
        model: resolve_job_model(config, model),
        mode,
        system_prompt,
        max_tokens,
        temperature,
        top_p,
        seed,
    }
}

fn resolve_job_model(config: &AgentConfig, requested_model: Option<String>) -> Option<String> {
    requested_model
        .map(|model| model.trim().to_string())
        .filter(|model| {
            !model.is_empty()
                && !model.eq_ignore_ascii_case("default")
                && !model.eq_ignore_ascii_case("auto")
        })
        .or_else(|| config.active_model.clone())
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

fn yellow(text: impl AsRef<str>) -> String {
    format!("\x1b[33m{}\x1b[0m", text.as_ref())
}

fn red(text: impl AsRef<str>) -> String {
    format!("\x1b[31m{}\x1b[0m", text.as_ref())
}

fn eprintln_error_field(label: &str, value: impl AsRef<str>) {
    eprintln!("{} {}", yellow(format!("{label}:")), red(value));
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
) -> Option<NodeAdmissionStatus> {
    let result = http::signed_post_json_body::<_, NodeAdmissionStatus>(
        &config.control_plane_url,
        "/v1/heartbeat",
        &config.device_id,
        identity,
        heartbeat,
    );
    match result {
        Ok(status) => {
            if verbose {
                println!("controlPlaneHeartbeat: ok");
            }
            Some(status)
        }
        Err(error) => {
            eprintln!("controlPlaneHeartbeat: {error}");
            if is_unknown_node_error(&error) {
                eprintln!("controlPlaneHeartbeat: re-registering missing node");
                let registration = build_registration(config, identity);
                if send_registration(config, identity, &registration, verbose) {
                    match http::signed_post_json_body::<_, NodeAdmissionStatus>(
                        &config.control_plane_url,
                        "/v1/heartbeat",
                        &config.device_id,
                        identity,
                        heartbeat,
                    ) {
                        Ok(status) => {
                            if verbose {
                                println!("controlPlaneHeartbeat: ok");
                            }
                            return Some(status);
                        }
                        Err(retry_error) => eprintln!("controlPlaneHeartbeat: {retry_error}"),
                    }
                }
            }
            None
        }
    }
}

fn launch_worker_process(
    config: &AgentConfig,
    request: WorkerLaunchRequest,
    json: bool,
    delta_sender: Option<mpsc::Sender<String>>,
) -> Result<contracts::WorkerLaunchResponse, String> {
    let model_dir = config.effective_model_dir();
    let (_, policy) = worker_readiness(config);
    if !policy.allowed {
        return Err(policy
            .reason
            .unwrap_or_else(|| "worker policy denied launch".to_string()));
    }

    match worker::launch_worker_with_stream(&request, &model_dir, delta_sender) {
        Ok(response) => {
            if json {
                emit_json_line(&response);
            } else {
                println!("workerLaunch: ok");
                println!("workerId: {}", response.worker_id);
                println!("jobId: {}", response.job_id);
                println!("status: {}", response.status);
                println!("backend: {}", response.backend);
                println!("{}", output_summary_line(&response.output));
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

fn output_summary_line(output: &str) -> String {
    let chars = output.chars().count();
    let first_line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .unwrap_or("");
    let preview: String = first_line.chars().take(160).collect();
    if chars > preview.chars().count() {
        format!("output: {chars} chars; preview: {preview}...")
    } else {
        format!("output: {chars} chars; preview: {preview}")
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
    println!(
        "runtimePreference: {}",
        health
            .runtime_preference
            .as_deref()
            .unwrap_or("platform-default")
    );
    println!(
        "fallbackRuntime: {}",
        health.fallback_runtime.as_deref().unwrap_or("none")
    );
    println!(
        "mlxAvailable: {}",
        if health.mlx_available { "yes" } else { "no" }
    );
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

fn relay_job_deltas(
    config: AgentConfig,
    identity: DeviceIdentity,
    job_id: String,
    assignment_id: String,
    deltas: mpsc::Receiver<String>,
) {
    let mut sequence = 1u64;
    while let Ok(delta) = deltas.recv() {
        let payload = JobStreamDelta {
            job_id: job_id.clone(),
            node_id: config.device_id.clone(),
            assignment_id: assignment_id.clone(),
            sequence,
            delta,
        };
        match signed_post_json_body::<_, JobStreamAck>(
            &config.control_plane_url,
            "/v1/jobs/delta",
            &config.device_id,
            &identity,
            &payload,
        ) {
            Ok(ack) if ack.accepted || ack.duplicate => sequence += 1,
            Ok(_) => {
                eprintln!("controlPlaneStream: delta {sequence} was not accepted; using final completion fallback");
                break;
            }
            Err(error) => {
                eprintln!("controlPlaneStream: {error}; using final completion fallback");
                break;
            }
        }
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

fn execute_claimed_job(
    config: AgentConfig,
    identity: DeviceIdentity,
    job: JobRecord,
    json: bool,
    _permit: local_api::SlotPermit,
) {
    let request = WorkerLaunchRequest {
        job_id: job.job_id.clone(),
        node_id: config.device_id.clone(),
        backend: job.backend.unwrap_or_else(|| resolved_backend(&config)),
        stream: job.stream,
        prompt: job.prompt.clone(),
        model: resolve_job_model(&config, job.model.clone()),
        mode: job.mode.clone(),
        system_prompt: job.system_prompt.clone(),
        max_tokens: job.max_tokens,
        temperature: job.temperature,
        top_p: job.top_p,
        seed: job.seed,
    };
    let (delta_sender, relay_handle) = if job.stream {
        if let Some(assignment_id) = job.assigned_at.clone() {
            let (sender, receiver) = mpsc::channel();
            let relay_config = config.clone();
            let relay_identity = identity.clone();
            let relay_job_id = job.job_id.clone();
            let handle = thread::spawn(move || {
                relay_job_deltas(
                    relay_config,
                    relay_identity,
                    relay_job_id,
                    assignment_id,
                    receiver,
                )
            });
            (Some(sender), Some(handle))
        } else {
            eprintln!("controlPlaneStream: assigned job has no assignment timestamp; using final completion fallback");
            (None, None)
        }
    } else {
        (None, None)
    };
    let started_at = Instant::now();
    let worker_result = launch_worker_process(&config, request, json, delta_sender);
    if let Some(handle) = relay_handle {
        let _ = handle.join();
    }
    match worker_result {
        Ok(response) => {
            let completion = build_completion_from_worker_response(
                response,
                started_at.elapsed().as_millis() as u64,
            );
            complete_job(&config, &identity, &completion);
        }
        Err(error) => {
            let completion = build_worker_error_completion(
                &job,
                &config,
                error,
                started_at.elapsed().as_millis() as u64,
            );
            complete_job(&config, &identity, &completion);
        }
    }
}

fn spawn_claimed_job(
    config: &AgentConfig,
    identity: &DeviceIdentity,
    job: JobRecord,
    json: bool,
    permit: local_api::SlotPermit,
) -> thread::JoinHandle<()> {
    let config = config.clone();
    let identity = identity.clone();
    thread::spawn(move || execute_claimed_job(config, identity, job, json, permit))
}

fn reap_finished_jobs(handles: &mut Vec<thread::JoinHandle<()>>) {
    let mut index = 0;
    while index < handles.len() {
        if handles[index].is_finished() {
            let handle = handles.swap_remove(index);
            let _ = handle.join();
        } else {
            index += 1;
        }
    }
}

fn process_pending_jobs(
    config: &AgentConfig,
    json: bool,
    verbose: bool,
    continuously_refill_slots: bool,
    slot_pool: Arc<local_api::SlotPool>,
) {
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
    let mut handles = Vec::new();
    while handles.len() < slot_pool.capacity() {
        let Some(permit) = slot_pool.try_acquire() else {
            break;
        };
        let Some(job) = claim_next_job(config, &identity) else {
            drop(permit);
            break;
        };
        println!("jobPoll: claimed {}", job.job_id);
        handles.push(spawn_claimed_job(config, &identity, job, json, permit));
    }
    if handles.is_empty() {
        if verbose {
            println!("jobPoll: none");
        }
        return;
    }

    let busy_heartbeat = build_heartbeat_with_state(config, AgentState::Busy);
    let _ = save_agent_state(&busy_heartbeat);
    let _ = save_heartbeat(&busy_heartbeat);
    send_heartbeat(config, &identity, &busy_heartbeat, verbose);
    let (stop_busy_heartbeat, busy_heartbeat_handle) =
        start_busy_heartbeat_supervisor(config.clone(), identity.clone(), Duration::from_secs(5));

    if continuously_refill_slots {
        while !handles.is_empty() {
            thread::sleep(Duration::from_secs(1));
            reap_finished_jobs(&mut handles);

            let refill_config = match load_agent_config() {
                Ok(Some(latest)) if should_agent_run(&latest) => Some(latest),
                Ok(_) => None,
                Err(error) => {
                    eprintln!("jobPoll: refill paused ({error})");
                    None
                }
            };
            if let Some(refill_config) = refill_config {
                while handles.len() < slot_pool.capacity() {
                    let Some(permit) = slot_pool.try_acquire() else {
                        break;
                    };
                    let Some(job) = claim_next_job(&refill_config, &identity) else {
                        drop(permit);
                        break;
                    };
                    println!("jobPoll: claimed {}", job.job_id);
                    handles.push(spawn_claimed_job(
                        &refill_config,
                        &identity,
                        job,
                        json,
                        permit,
                    ));
                }
            }
        }
    } else {
        for handle in handles {
            let _ = handle.join();
        }
    }
    stop_busy_heartbeat_supervisor(stop_busy_heartbeat, busy_heartbeat_handle);

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
    should_agent_run(config)
}

fn should_agent_run(config: &AgentConfig) -> bool {
    config.connected && !config.paused
}

fn clear_runtime_environment() {
    std::env::remove_var("OPENGPU_LLAMA_SERVER_URL");
    std::env::remove_var("OPENGPU_MLX_SERVER_URL");
    std::env::remove_var("OPENGPU_VLLM_URL");
}

fn run_agent(once: bool, json: bool, verbose: bool, interval_seconds: u64) {
    let config = load_config_or_exit();
    let identity = load_identity_or_exit();
    let runtime_parallel_slots = worker_readiness(&config).0.parallel_slots;
    let mut persistent_runtime = if json || !should_keep_runtime_warm(&config) {
        None
    } else {
        match worker::start_persistent_runtime(
            &config.effective_model_dir(),
            config.active_model.as_deref(),
            resolved_backend(&config),
            runtime_parallel_slots,
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("persistentRuntime: unavailable ({error}); falling back to batch");
                None
            }
        }
    };
    if let Some(runtime) = persistent_runtime.as_ref() {
        std::env::set_var(runtime.environment_variable(), runtime.url());
    }
    let slot_pool = local_api::SlotPool::new(usize::from(runtime_parallel_slots.max(1)));
    let _local_api = if json || !local_api::enabled() {
        None
    } else {
        match local_api::start(
            config.clone(),
            identity.clone(),
            resolved_backend(&config),
            slot_pool.clone(),
        ) {
            Ok(handle) => Some(handle),
            Err(error) => {
                eprintln!("localApi: unavailable ({error})");
                None
            }
        }
    };
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
    let control_plane_status = send_heartbeat(&config, &identity, &heartbeat, verbose);
    print_capability_summary_with_control_plane(
        &registration.capabilities,
        control_plane_status.as_ref(),
    );
    if let Some(reason) = control_plane_blocks_jobs(control_plane_status.as_ref()) {
        drop(persistent_runtime.take());
        clear_runtime_environment();
        eprintln_error_field("agentAdmission", "blocked by control plane");
        eprintln_error_field("agentAdmissionReason", reason);
        eprintln_error_field("persistentRuntime", "stopped");
        std::process::exit(2);
    }
    process_pending_jobs(&config, json, verbose, !once, slot_pool.clone());

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
                clear_runtime_environment();
                break;
            }
            Err(error) => {
                eprintln!(
                    "agentStop: failed to reload config ({error}); cooling persistent runtime"
                );
                drop(persistent_runtime.take());
                clear_runtime_environment();
                break;
            }
        };
        if !should_agent_run(&latest_config) {
            let heartbeat = build_heartbeat(&latest_config);
            let _ = save_agent_state(&heartbeat);
            let _ = save_heartbeat(&heartbeat);
            send_heartbeat(&latest_config, &identity, &heartbeat, verbose);
            drop(persistent_runtime.take());
            clear_runtime_environment();
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
        process_pending_jobs(&latest_config, json, verbose, true, slot_pool.clone());
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
    print_capability_summary(&registration.capabilities);
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
    println!(
        "policyAllowed: {}",
        if heartbeat.policy_allowed {
            "yes"
        } else {
            "no"
        }
    );
    if let Some(reason) = heartbeat.policy_reason.as_ref() {
        println!("policyReason: {reason}");
    }
    print_capability_summary(&heartbeat.capabilities);
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

    if let Err(error) = save_agent_config(&config) {
        eprintln!("failed to save stopped config: {error}");
        std::process::exit(1);
    }

    if let Err(error) = save_agent_state(&heartbeat) {
        eprintln!("failed to save stopped state: {error}");
        std::process::exit(1);
    }

    let identity = load_identity_or_exit();
    send_heartbeat(&config, &identity, &heartbeat, false);

    println!("{}", red(format!("disconnected {}", config.device_id)));
    println!("connected: no");
    println!("paused: yes");
    match worker::stop_persistent_runtime_from_state() {
        Ok(Some(runtime)) => {
            println!("persistentRuntime: stopped");
            println!("persistentRuntimePid: {}", runtime.pid);
        }
        Ok(None) => println!("persistentRuntime: not running"),
        Err(error) => {
            eprintln!("persistentRuntimeStop: {error}");
            std::process::exit(1);
        }
    }
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
            mode,
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
                mode,
                system_prompt,
                max_tokens,
                temperature,
                top_p,
                seed,
            );
            let _ = launch_worker_process(&config, request, json, None);
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
    fn reaps_finished_jobs_without_waiting_for_active_workers() {
        let completed = thread::spawn(|| {});
        while !completed.is_finished() {
            thread::yield_now();
        }
        let (release_tx, release_rx) = mpsc::channel();
        let active = thread::spawn(move || {
            release_rx.recv().expect("release active worker");
        });
        let mut handles = vec![completed, active];

        reap_finished_jobs(&mut handles);

        assert_eq!(handles.len(), 1);
        release_tx.send(()).expect("release worker");
        handles.pop().unwrap().join().unwrap();
    }

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
            runtime_preference: None,
            fallback_runtime: None,
            contributed_cluster: None,
            cluster_prompt_declined: false,
            max_jobs: None,
        }
    }

    #[test]
    fn registration_advertises_capability_fabric_v1() {
        let identity = DeviceIdentity {
            public_key_hex: "0011".to_string(),
            private_key_hex: String::new(),
            fingerprint: "fingerprint".to_string(),
            keychain_label_hex: None,
            encrypted_private_key_hex: String::new(),
            nonce_hex: String::new(),
        };

        let registration = build_registration(&test_config(), &identity);

        assert_eq!(registration.capability_fabric_version, "1.0");
        assert_eq!(registration.backend, registration.capabilities.backend);
        assert_eq!(
            registration.contribution_percent,
            registration.capabilities.contribution_percent
        );
        assert!(registration.capabilities.schema_version > 0);
    }

    /// Health as `contributed_cluster_health` reports it: endpoint reachable, no
    /// local model file, no llama runtime.
    fn cluster_health(model: &str) -> WorkerHealthReport {
        let mut health = test_health(Backend::Cuda);
        health.model_name = Some(model.to_string());
        health.model_path = None;
        health.llama_cli_available = false;
        health.persistent_runtime_warm = true;
        health.persistent_runtime_url = Some("http://127.0.0.1:8000".to_string());
        health.runtime_kind = "contributed-cluster".to_string();
        health.runtime_mode = "contributed-cluster".to_string();
        health.cuda_memory_mb = None;
        health
    }

    fn cluster_config(model: &str, params: Option<u64>, bytes: Option<u64>) -> AgentConfig {
        let mut config = test_config();
        config.active_model = None;
        config.models = Vec::new();
        config.contributed_cluster = Some(crate::storage::ContributedCluster {
            kind: "vllm".to_string(),
            base_url: "http://127.0.0.1:8000".to_string(),
            capacity_class: "server".to_string(),
            models: vec![model.to_string()],
            model: Some(model.to_string()),
            model_params: params,
            model_bytes: bytes,
            model_capabilities: Vec::new(),
            model_context_tokens: None,
            memory_utilization: None,
            max_concurrency: None,
            max_num_seqs: None,
            kv_cache_tokens: None,
            supports_tool_calls: false,
            adopted_at: Some("1".to_string()),
        });
        config
    }

    #[test]
    fn a_contributed_cluster_advertises_its_model_without_a_local_cache() {
        let config = cluster_config("UD-IQ2_M", Some(753_864_139_008), Some(238_568_039_424));
        let health = cluster_health("UD-IQ2_M");

        let capabilities = build_capabilities(&config, &health, true);

        assert_eq!(
            capabilities
                .active_model
                .as_ref()
                .map(|model| model.name.as_str()),
            Some("UD-IQ2_M")
        );
        // Nothing is cached locally, so there is no model path to advertise.
        assert!(capabilities
            .active_model
            .as_ref()
            .and_then(|model| model.path.as_ref())
            .is_none());
        assert!(capabilities.ready_for_jobs);
    }

    #[test]
    fn a_contributed_cluster_uses_its_persisted_server_capacity() {
        let config = cluster_config("UD-IQ2_M", Some(753_864_139_008), None);
        let capabilities = build_capabilities(&config, &cluster_health("UD-IQ2_M"), true);

        assert_eq!(capabilities.capacity_class, "server");
    }

    #[test]
    fn the_contribution_cap_does_not_produce_a_vram_budget_for_a_cluster() {
        let mut config = cluster_config("UD-IQ2_M", Some(753_864_139_008), None);
        config.contribution_percent = 30;
        let mut health = cluster_health("UD-IQ2_M");
        health.cuda_memory_mb = Some(24_576);

        let capabilities = build_capabilities(&config, &health, true);

        assert_eq!(capabilities.usable_vram_mb, None);
        // The cap is still reported so the control plane can see what was chosen.
        assert_eq!(capabilities.contribution_percent, 30);
    }

    #[test]
    fn a_cluster_without_a_model_is_not_advertised_ready() {
        let mut config = cluster_config("UD-IQ2_M", None, None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            cluster.model = None;
            cluster.models = Vec::new();
        }

        let capabilities = build_capabilities(&config, &cluster_health("UD-IQ2_M"), true);

        assert!(!capabilities.ready_for_jobs);
        assert_eq!(
            capabilities.readiness_reason.as_deref(),
            Some("contributed cluster advertises no model")
        );
    }

    #[test]
    fn a_cluster_advertising_tools_reports_tool_support() {
        let mut config = cluster_config("hermes3:70b", Some(70_600_000_000), None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            cluster.model_capabilities = vec!["completion".to_string(), "tools".to_string()];
            cluster.model_context_tokens = Some(131_072);
        }

        let health = cluster_health("hermes3:70b");
        let capabilities = build_capabilities(&config, &health, true);
        let profile = build_scheduler_capabilities(&config, &health, &capabilities, 8_192, 100);

        assert!(profile.supports_tools);
        assert!(!profile.supports_embeddings);
        // The reported context length beats the name-based heuristic.
        assert_eq!(profile.max_context_tokens, Some(131_072));
        assert_eq!(profile.models[0].max_output_tokens, None);
        assert_eq!(
            profile.models[0].output_capacity_mode.as_deref(),
            Some("context_window")
        );
        assert!(profile.supported_tools.contains(&"tool_use".to_string()));
    }

    #[test]
    fn contributed_cluster_advertises_context_bound_output_and_raw_capacity() {
        let mut config = cluster_config("qwen3-coder", Some(30_000_000_000), None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            cluster.model_context_tokens = Some(4_096);
            cluster.max_num_seqs = Some(32);
            cluster.kv_cache_tokens = Some(9_435_151);
        }

        let health = cluster_health("qwen3-coder");
        let capabilities = build_capabilities(&config, &health, true);
        let profile = build_scheduler_capabilities(&config, &health, &capabilities, 8_192, 100);

        assert_eq!(profile.models[0].context_tokens, Some(4_096));
        assert_eq!(profile.models[0].max_output_tokens, None);
        assert_eq!(
            profile.models[0].output_capacity_mode.as_deref(),
            Some("context_window")
        );
        assert_eq!(profile.max_num_seqs, Some(32));
        assert_eq!(profile.kv_cache_size_tokens, Some(9_435_151));
    }

    #[test]
    fn a_cluster_without_tool_capability_does_not_claim_it() {
        // llama.cpp reports only ["completion"] for this model.
        let mut config = cluster_config("UD-IQ2_M", Some(753_864_139_008), None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            cluster.model_capabilities = vec!["completion".to_string()];
        }

        let health = cluster_health("UD-IQ2_M");
        let capabilities = build_capabilities(&config, &health, true);
        let profile = build_scheduler_capabilities(&config, &health, &capabilities, 8_192, 100);

        assert!(!profile.supports_tools);
        assert!(!profile.supports_vision);
    }

    #[test]
    fn a_cluster_serving_an_embedding_model_reports_embeddings() {
        let mut config = cluster_config("nomic-embed-text", Some(137_000_000), None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            cluster.model_capabilities = vec!["embedding".to_string()];
        }

        let health = cluster_health("nomic-embed-text");
        let capabilities = build_capabilities(&config, &health, true);
        let profile = build_scheduler_capabilities(&config, &health, &capabilities, 8_192, 100);

        assert!(profile.supports_embeddings);
    }

    #[test]
    fn usable_memory_comes_from_the_machine_not_from_free_memory() {
        // The bug this fixes: a 121.6 GB node serving a 75 GB model had only
        // ~6.9 GB free, so min(cap, available) classified it as `micro`.
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        cluster.memory_utilization = Some(0.85);

        let usable = contributed_cluster_usable_memory_mb(124_545, &cluster);

        assert_eq!(usable, 105_863); // 124_545 x 0.85
        assert_eq!(classify_capacity(Backend::Cuda, usable, None), "synthesis");
    }

    #[test]
    fn usable_memory_falls_back_to_eighty_five_percent() {
        let cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        assert_eq!(cluster.memory_utilization, None);

        // No reported share, so assume the conventional 0.85.
        assert_eq!(
            contributed_cluster_usable_memory_mb(100_000, &cluster),
            85_000
        );
    }

    #[test]
    fn a_nonsense_utilization_share_is_ignored() {
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        for bad in [0.0, -0.5, 1.5] {
            cluster.memory_utilization = Some(bad);
            assert_eq!(
                contributed_cluster_usable_memory_mb(100_000, &cluster),
                85_000
            );
        }
    }

    #[test]
    fn the_contributors_limit_wins_over_runtime_ceilings() {
        // The runtime can hold far more than the contributor wants to give.
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        cluster.max_concurrency = Some(71);
        cluster.max_num_seqs = Some(32);
        let mut config = cluster_config("qwen3-coder", None, None);
        config.contributed_cluster = Some(cluster);
        config.max_jobs = Some(8);

        assert_eq!(worker_readiness(&config).0.parallel_slots, 8);
    }

    #[test]
    fn a_contributors_limit_of_one_is_respected() {
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        cluster.max_concurrency = Some(71);
        let mut config = cluster_config("qwen3-coder", None, None);
        config.contributed_cluster = Some(cluster);
        config.max_jobs = Some(1);

        assert_eq!(worker_readiness(&config).0.parallel_slots, 1);
    }

    #[test]
    fn without_a_contributor_limit_the_runtime_ceilings_still_apply() {
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        cluster.max_concurrency = Some(71);

        // Falls back to the unverified cap rather than donating everything.
        assert_eq!(contributed_cluster_slots(&cluster), 16);
    }

    #[test]
    fn a_zero_contributor_limit_is_ignored_rather_than_disabling_the_node() {
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");
        cluster.max_concurrency = Some(71);
        let mut config = cluster_config("qwen3-coder", None, None);
        config.contributed_cluster = Some(cluster);
        // Zero would silently disable the node, so it is ignored.
        config.max_jobs = Some(0);

        assert_eq!(worker_readiness(&config).0.parallel_slots, 16);
    }

    #[test]
    fn slots_come_from_the_runtime_and_stay_bounded() {
        let mut cluster = cluster_config("qwen3-coder", None, None)
            .contributed_cluster
            .expect("cluster");

        // Nothing reported: one slot, as before.
        assert_eq!(contributed_cluster_slots(&cluster), 1);

        cluster.max_concurrency = Some(8);
        assert_eq!(contributed_cluster_slots(&cluster), 8);

        // Without a scheduler limit, theoretical KV capacity stays safety-bounded.
        cluster.max_concurrency = Some(71);
        assert_eq!(contributed_cluster_slots(&cluster), 16);

        // Once max_num_seqs is known, it is the runtime's authoritative
        // scheduler ceiling, still bounded by the available KV cache.
        cluster.max_num_seqs = Some(32);
        assert_eq!(contributed_cluster_slots(&cluster), 32);
        cluster.max_concurrency = Some(25);
        assert_eq!(contributed_cluster_slots(&cluster), 25);
    }

    #[test]
    fn a_loaded_tool_parser_makes_the_node_tool_capable() {
        let mut config = cluster_config("qwen3-coder", Some(80_000_000_000), None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            // vLLM publishes no capability array; the parser metric is the signal.
            cluster.model_capabilities = Vec::new();
            cluster.supports_tool_calls = true;
        }
        let health = cluster_health("qwen3-coder");
        let capabilities = build_capabilities(&config, &health, true);
        let profile = build_scheduler_capabilities(&config, &health, &capabilities, 8_192, 100);

        assert!(profile.supports_tools);
        assert!(profile.supported_tools.contains(&"tool_use".to_string()));
    }

    #[test]
    fn a_node_without_a_cluster_also_honours_the_contributors_limit() {
        // The memory ladder caps a managed runtime at 4 slots; a contributor
        // donating a large machine can say otherwise.
        let mut config = test_config();
        config.contributed_cluster = None;
        config.max_jobs = Some(6);

        assert_eq!(worker_readiness(&config).0.parallel_slots, 6);
    }

    #[test]
    fn capacity_rank_orders_the_ladder() {
        assert!(capacity_rank("server") > capacity_rank("synthesis"));
        assert!(capacity_rank("synthesis") > capacity_rank("heavy"));
        assert!(capacity_rank("heavy") > capacity_rank("standard"));
        assert_eq!(capacity_rank("nonsense"), 0);
    }

    #[test]
    fn a_big_contributed_cluster_earns_reducer_and_synthesizer_roles() {
        // Without this, the control plane silently skips the node for the heavy
        // graph nodes: it gates those two roles strictly on role membership.
        let config = cluster_config("UD-IQ2_M", Some(753_864_139_008), None);
        let health = cluster_health("UD-IQ2_M");
        let capabilities = build_capabilities(&config, &health, true);

        let roles = node_roles_for(resolved_backend(&config), &health, &capabilities, 5_400);

        assert_eq!(capabilities.capacity_class, "server");
        assert!(roles.contains(&NodeRole::Synthesizer));
        assert!(roles.contains(&NodeRole::Reducer));
        assert!(roles.contains(&NodeRole::Coding));
    }

    #[test]
    fn every_contributed_cluster_claims_server_roles() {
        let config = cluster_config("hermes3:8b", Some(8_000_000_000), None);
        let health = cluster_health("hermes3:8b");
        let capabilities = build_capabilities(&config, &health, true);

        let roles = node_roles_for(Backend::Auto, &health, &capabilities, 4_096);

        assert_eq!(capabilities.capacity_class, "server");
        assert!(roles.contains(&NodeRole::Synthesizer));
        assert!(roles.contains(&NodeRole::Reducer));
        assert!(roles.contains(&NodeRole::Chat));
    }

    #[test]
    fn omitted_and_default_job_models_use_the_active_model() {
        let config = test_config();
        assert_eq!(
            resolve_job_model(&config, None).as_deref(),
            Some("tiny-cuda")
        );
        assert_eq!(
            resolve_job_model(&config, Some("default".to_string())).as_deref(),
            Some("tiny-cuda")
        );
        assert_eq!(
            resolve_job_model(&config, Some(" auto ".to_string())).as_deref(),
            Some("tiny-cuda")
        );
    }

    #[test]
    fn explicit_job_model_is_preserved_and_trimmed() {
        let config = test_config();
        assert_eq!(
            resolve_job_model(&config, Some(" Qwen/Explicit ".to_string())).as_deref(),
            Some("Qwen/Explicit")
        );
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

        config.connected = true;
        config.backend_preference = Backend::Vllm;
        assert!(should_keep_runtime_warm(&config));

        config.backend_preference = Backend::M;
        config.runtime_preference = Some("mlx".to_string());
        assert!(should_agent_run(&config));
        assert!(should_keep_runtime_warm(&config));

        config.runtime_preference = Some("llama-metal".to_string());
        assert!(should_keep_runtime_warm(&config));
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
            runtime_preference: None,
            fallback_runtime: None,
            mlx_available: false,
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
            parallel_slots: 1,
            supported_runtime_modes: vec!["local".to_string()],
            streaming_supported: false,
            capabilities: NodeCapabilityProfile::default(),
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
            stream: false,
            model: Some("tiny-cuda".to_string()),
            mode: None,
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
    fn admission_status_blocks_startup_when_control_plane_denies_jobs() {
        let status = NodeAdmissionStatus {
            node_id: "node-1".to_string(),
            state: AgentState::Ready,
            policy_allowed: false,
            policy_reason: Some("CUDA VRAM below minimum".to_string()),
            computed_policy_allowed: false,
            computed_policy_reason: None,
        };

        assert_eq!(
            control_plane_blocks_jobs(Some(&status)).as_deref(),
            Some("CUDA VRAM below minimum")
        );
    }

    #[test]
    fn admission_status_allows_startup_when_control_plane_allows_jobs() {
        let status = NodeAdmissionStatus {
            node_id: "node-1".to_string(),
            state: AgentState::Ready,
            policy_allowed: true,
            policy_reason: None,
            computed_policy_allowed: true,
            computed_policy_reason: None,
        };

        assert_eq!(control_plane_blocks_jobs(Some(&status)), None);
        assert_eq!(control_plane_blocks_jobs(None), None);
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
        assert_eq!(capability.schema_version, 4);
        assert!(capability.physical_memory_mb.is_some());
        assert!(capability.usable_memory_mb.is_some());
        assert!(capability.available_memory_mb.is_some());
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn normalized_capacity_classes_cover_supported_machine_shapes() {
        assert_eq!(classify_capacity(Backend::Auto, 8_192, None), "standard");
        assert_eq!(classify_capacity(Backend::M, 16_384, None), "performance");
        assert_eq!(
            classify_capacity(Backend::Cuda, 24_576, Some(24_576)),
            "heavy"
        );
        assert_eq!(
            classify_capacity(Backend::Cuda, 49_152, Some(49_152)),
            "synthesis"
        );
        assert_eq!(classify_capacity(Backend::M, 65_536, None), "synthesis");
        assert_eq!(classify_capacity(Backend::Vllm, 8_192, None), "server");
    }

    #[test]
    fn scheduler_capability_profile_advertises_roles_and_budget() {
        let mut config = test_config();
        let temp = std::env::temp_dir().join(format!(
            "opengpu-scheduler-capability-test-{}",
            now_unix_seconds()
        ));
        config.model_dir = Some(temp.display().to_string());
        config.contribution_percent = 50;
        write_active_model_manifest(&config, "accepted");
        let mut health = test_health(Backend::Cuda);
        health.parallel_slots = 2;
        let capability = build_capabilities(&config, &health, true);

        let scheduler_capability =
            build_scheduler_capabilities(&config, &health, &capability, 32_768, 75);

        assert_eq!(scheduler_capability.models.len(), 1);
        assert_eq!(scheduler_capability.models[0].name, "tiny-cuda");
        assert_eq!(scheduler_capability.max_context_tokens, Some(4_096));
        assert_eq!(scheduler_capability.total_vram_mb, Some(4096));
        assert_eq!(scheduler_capability.available_vram_mb, Some(2048));
        assert_eq!(scheduler_capability.max_parallel_jobs, 2);
        assert_eq!(scheduler_capability.current_load_percent, Some(25));
        assert!(scheduler_capability.roles.contains(&NodeRole::Chat));
        assert!(scheduler_capability.roles.contains(&NodeRole::Coding));
        assert!(scheduler_capability
            .roles
            .contains(&NodeRole::ChunkAnalysis));
        assert!(!scheduler_capability.roles.contains(&NodeRole::Reducer));
        assert!(scheduler_capability.roles.contains(&NodeRole::Batch));
        assert_eq!(
            scheduler_capability.skill_tags,
            vec!["backend:cuda".to_string(), "runtime:cuda".to_string()]
        );
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn sixteen_gb_cuda_node_does_not_overstate_a_small_models_roles() {
        let mut config = test_config();
        let temp = std::env::temp_dir().join(format!(
            "opengpu-synthesis-capability-test-{}",
            now_unix_seconds()
        ));
        config.model_dir = Some(temp.display().to_string());
        config.contribution_percent = 100;
        write_active_model_manifest(&config, "accepted");
        let mut health = test_health(Backend::Cuda);
        health.cuda_memory_mb = Some(16_384);
        let capability = build_capabilities(&config, &health, true);

        let scheduler_capability =
            build_scheduler_capabilities(&config, &health, &capability, 32_768, 100);

        assert!(!scheduler_capability.roles.contains(&NodeRole::Synthesizer));
        assert!(!scheduler_capability.roles.contains(&NodeRole::Reducer));
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn contributed_cluster_advertises_each_model_with_cumulative_capabilities() {
        let mut config = cluster_config("qwen2.5:3b", Some(70_000_000_000), None);
        if let Some(cluster) = config.contributed_cluster.as_mut() {
            cluster.models = vec!["qwen2.5:3b".to_string(), "qwen2.5:70b".to_string()];
            cluster.model = Some("qwen2.5:3b".to_string());
            cluster.model_capabilities = vec!["tools".to_string()];
        }
        let health = cluster_health("qwen2.5:3b");
        let capability = build_capabilities(&config, &health, true);

        let profile = build_scheduler_capabilities(&config, &health, &capability, 8_192, 100);

        assert_eq!(profile.models.len(), 2);
        let small = profile
            .models
            .iter()
            .find(|model| model.name == "qwen2.5:3b")
            .unwrap();
        let large = profile
            .models
            .iter()
            .find(|model| model.name == "qwen2.5:70b")
            .unwrap();
        assert!(small.active);
        assert!(small.warm);
        assert!(small
            .task_capabilities
            .contains(&"small_coding".to_string()));
        assert!(!small
            .task_capabilities
            .contains(&"large_coding".to_string()));
        assert!(large
            .task_capabilities
            .contains(&"small_coding".to_string()));
        assert!(large
            .task_capabilities
            .contains(&"large_coding".to_string()));
        assert!(large.task_capabilities.contains(&"synthesizer".to_string()));
        assert!(profile.roles.contains(&NodeRole::Synthesizer));
    }

    #[test]
    fn unready_node_advertises_no_scheduler_roles() {
        let config = test_config();
        let health = test_health(Backend::Cuda);
        let capability = build_capabilities(&config, &health, false);

        let scheduler_capability =
            build_scheduler_capabilities(&config, &health, &capability, 32_768, 80);

        assert!(!capability.ready_for_jobs);
        assert!(scheduler_capability.roles.is_empty());
        assert_eq!(scheduler_capability.models.len(), 1);
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
