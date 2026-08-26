use crate::contracts::{
    Backend, LocalSlotLeaseRecord, LocalSlotLeaseReleaseRequest, LocalSlotLeaseRenewRequest,
    LocalSlotLeaseRequest, LocalSlotLeaseResponse, WorkerLaunchRequest,
};
use crate::http::signed_post_json_body;
use crate::identity::DeviceIdentity;
use crate::storage::{local_api_token_path, AgentConfig};
use crate::worker;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const DEFAULT_LOCAL_API_ADDR: &str = "127.0.0.1:11435";

pub fn enabled() -> bool {
    !std::env::var("OPENGPU_LOCAL_FIRST_ENABLED")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
}
const MAX_LOCAL_BODY_BYTES: u64 = 64 * 1024;
const LOCAL_LEASE_TTL_SECONDS: u64 = 120;
const LOCAL_LEASE_RENEW_SECONDS: u64 = 30;

#[derive(Debug)]
pub struct SlotPool {
    capacity: usize,
    active: AtomicUsize,
}

impl SlotPool {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity: capacity.max(1),
            active: AtomicUsize::new(0),
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub fn available(&self) -> usize {
        self.capacity.saturating_sub(self.active())
    }

    pub fn try_acquire(self: &Arc<Self>) -> Option<SlotPermit> {
        let mut active = self.active.load(Ordering::Acquire);
        loop {
            if active >= self.capacity {
                return None;
            }
            match self.active.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(SlotPermit { pool: self.clone() }),
                Err(observed) => active = observed,
            }
        }
    }
}

#[derive(Debug)]
pub struct SlotPermit {
    pool: Arc<SlotPool>,
}

impl Drop for SlotPermit {
    fn drop(&mut self) {
        self.pool.active.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct LocalApiHandle {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for LocalApiHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug, Deserialize)]
struct LocalInferenceRequest {
    #[serde(default)]
    request_id: Option<String>,
    prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    backend: Option<Backend>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    force_local: bool,
}

#[derive(Debug, Serialize)]
struct LocalInferenceResponse {
    request_id: String,
    routing: String,
    node_id: String,
    model: Option<String>,
    output: String,
    status: String,
    error: Option<String>,
}

fn local_api_addr() -> String {
    std::env::var("OPENGPU_LOCAL_AGENT_ADDR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| value.starts_with("127.0.0.1:") || value.starts_with("[::1]:"))
        .unwrap_or_else(|| DEFAULT_LOCAL_API_ADDR.to_string())
}

fn load_or_create_token() -> Result<String, String> {
    let path = local_api_token_path();
    if let Ok(token) = fs::read_to_string(&path) {
        let token = token.trim();
        if token.len() >= 32 {
            return Ok(token.to_string());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let token = format!(
        "local-{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    fs::write(&path, format!("{token}\n")).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(token)
}

fn bearer_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .and_then(|header| header.value.as_str().strip_prefix("Bearer "))
}

fn json_response(status: u16, value: serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(&value).expect("local API JSON");
    let content_type = Header::from_bytes("Content-Type", "application/json; charset=utf-8")
        .expect("content type header");
    Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(content_type)
}

fn read_json<T: for<'de> Deserialize<'de>>(request: &mut Request) -> Result<T, String> {
    let mut body = String::new();
    request
        .as_reader()
        .take(MAX_LOCAL_BODY_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() as u64 > MAX_LOCAL_BODY_BYTES {
        return Err("local request body is too large".to_string());
    }
    serde_json::from_str(&body).map_err(|error| error.to_string())
}

fn parameter_count_from_model_name(model: &str) -> Option<u64> {
    let lower = model.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'b' || index == 0 {
            continue;
        }
        let mut start = index;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        if start == index {
            continue;
        }
        let billions = lower[start..index].parse::<f64>().ok()?;
        if (0.1..=1000.0).contains(&billions) {
            return Some((billions * 1_000_000_000.0) as u64);
        }
    }
    None
}

fn active_model_parameter_count(config: &AgentConfig, model: Option<&str>) -> Option<u64> {
    let cluster_params = config.contributed_cluster.as_ref().and_then(|cluster| {
        let same_model = match (model, cluster.model.as_deref()) {
            (Some(requested), Some(active)) => requested.eq_ignore_ascii_case(active),
            _ => true,
        };
        same_model.then_some(cluster.model_params).flatten()
    });
    cluster_params.or_else(|| model.and_then(parameter_count_from_model_name))
}

fn local_suitability_error(
    config: &AgentConfig,
    model: Option<&str>,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Option<String> {
    let parameters = active_model_parameter_count(config, model)?;
    let lower = prompt.to_ascii_lowercase();
    let substantial_code = [
        "complete code",
        "complete program",
        "entire program",
        "full program",
        "production-ready",
        "implement ",
        "debug ",
        "crud api",
        "architecture",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let deep_reasoning = [
        "deep dive",
        "comprehensive",
        "detailed analysis",
        "step by step reasoning",
        "prove that",
        "research",
        "synthesize",
        "compare and recommend",
        "multi-step",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let long_context = prompt.chars().count() > 4_000;
    let large_output = max_tokens.unwrap_or_default() > 1_024;

    if parameters <= 4_000_000_000
        && (substantial_code || deep_reasoning || long_context || large_output)
    {
        let reason = if substantial_code {
            "substantial code generation"
        } else if deep_reasoning {
            "complex reasoning or synthesis"
        } else if long_context {
            "long-context processing"
        } else {
            "a large output budget"
        };
        let model = model.unwrap_or("the active local model");
        return Some(format!(
            "local model `{model}` is a small model and the request requires {reason}; use the control-plane planner"
        ));
    }
    None
}

fn control_plane_is_unreachable(error: &str) -> bool {
    error.starts_with("transport failed:")
}

fn start_lease_manager(
    config: AgentConfig,
    identity: DeviceIdentity,
    request: LocalSlotLeaseRequest,
    initial_lease: Option<LocalSlotLeaseRecord>,
) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut lease = initial_lease;
        loop {
            let retry_seconds = if lease.is_some() {
                LOCAL_LEASE_RENEW_SECONDS
            } else {
                5
            };
            match stop_rx.recv_timeout(Duration::from_secs(retry_seconds)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(current) = lease.as_ref() {
                let payload = LocalSlotLeaseRenewRequest {
                    lease_id: current.lease_id.clone(),
                    ttl_seconds: LOCAL_LEASE_TTL_SECONDS,
                };
                match signed_post_json_body::<_, LocalSlotLeaseResponse>(
                    &config.control_plane_url,
                    "/v1/local-leases/renew",
                    &config.device_id,
                    &identity,
                    &payload,
                ) {
                    Ok(response) => {
                        lease = response.lease.or(lease);
                    }
                    Err(error) => {
                        eprintln!("localLeaseRenewal: {error}");
                        lease = None;
                    }
                }
            } else {
                match signed_post_json_body::<_, LocalSlotLeaseResponse>(
                    &config.control_plane_url,
                    "/v1/local-leases",
                    &config.device_id,
                    &identity,
                    &request,
                ) {
                    Ok(response) if response.granted => {
                        lease = response.lease;
                        eprintln!("localLeaseRecovery: local occupancy synchronized");
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("localLeaseRecovery: {error}"),
                }
            }
        }
        if let Some(lease) = lease {
            let _ = signed_post_json_body::<_, LocalSlotLeaseResponse>(
                &config.control_plane_url,
                "/v1/local-leases/release",
                &config.device_id,
                &identity,
                &LocalSlotLeaseReleaseRequest {
                    lease_id: lease.lease_id,
                },
            );
        }
    });
    (stop_tx, handle)
}

fn handle_local_inference(
    mut request: Request,
    config: &AgentConfig,
    identity: &DeviceIdentity,
    backend: Backend,
    slots: &Arc<SlotPool>,
) {
    let payload = match read_json::<LocalInferenceRequest>(&mut request) {
        Ok(payload) => payload,
        Err(error) => {
            let _ = request.respond(json_response(400, serde_json::json!({ "error": error })));
            return;
        }
    };
    if payload.prompt.trim().is_empty() {
        let _ = request.respond(json_response(
            400,
            serde_json::json!({ "error": "prompt is required" }),
        ));
        return;
    }
    let active_model = config.active_model.clone();
    if let Some(requested) = payload.model.as_deref() {
        if active_model
            .as_deref()
            .is_some_and(|active| !active.eq_ignore_ascii_case(requested))
        {
            let _ = request.respond(json_response(
                409,
                serde_json::json!({ "error": "requested model is not active locally", "fallback": "network" }),
            ));
            return;
        }
    }
    if payload
        .backend
        .is_some_and(|requested| requested != Backend::Auto && requested != backend)
    {
        let _ = request.respond(json_response(
            409,
            serde_json::json!({ "error": "requested backend is not active locally", "fallback": "network" }),
        ));
        return;
    }
    let Some(_permit) = slots.try_acquire() else {
        let _ = request.respond(json_response(
            409,
            serde_json::json!({ "error": "local capacity is unavailable", "fallback": "network" }),
        ));
        return;
    };
    let request_id = payload
        .request_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("local-{}", uuid::Uuid::new_v4().simple()));
    let model = payload.model.or(active_model);
    let lease_request = LocalSlotLeaseRequest {
        request_id: request_id.clone(),
        slots: 1,
        model: model.clone(),
        ttl_seconds: LOCAL_LEASE_TTL_SECONDS,
    };
    let (lease, offline) = match signed_post_json_body::<_, LocalSlotLeaseResponse>(
        &config.control_plane_url,
        "/v1/local-leases",
        &config.device_id,
        identity,
        &lease_request,
    ) {
        Ok(response) if response.granted => (response.lease, false),
        Ok(response) => {
            let _ = request.respond(json_response(
                409,
                serde_json::json!({ "error": response.error.unwrap_or_else(|| "local lease denied".to_string()), "fallback": "network" }),
            ));
            return;
        }
        Err(error) if control_plane_is_unreachable(&error) => {
            eprintln!("localOfflineMode: {error}");
            (None, true)
        }
        Err(error) => {
            let _ = request.respond(json_response(
                503,
                serde_json::json!({ "error": format!("control-plane lease rejected: {error}"), "fallback": "network" }),
            ));
            return;
        }
    };
    if !offline && lease.is_none() {
        let _ = request.respond(json_response(
            503,
            serde_json::json!({ "error": "control plane returned no lease" }),
        ));
        return;
    }
    if !payload.force_local && !offline {
        if let Some(error) = local_suitability_error(
            config,
            model.as_deref(),
            &payload.prompt,
            payload.max_tokens,
        ) {
            if let Some(lease) = lease {
                let _ = signed_post_json_body::<_, LocalSlotLeaseResponse>(
                    &config.control_plane_url,
                    "/v1/local-leases/release",
                    &config.device_id,
                    identity,
                    &LocalSlotLeaseReleaseRequest {
                        lease_id: lease.lease_id,
                    },
                );
            }
            let _ = request.respond(json_response(
                409,
                serde_json::json!({ "error": error, "fallback": "network" }),
            ));
            return;
        }
    }
    let (lease_stop, lease_handle) =
        start_lease_manager(config.clone(), identity.clone(), lease_request, lease);
    let worker_request = WorkerLaunchRequest {
        job_id: request_id.clone(),
        node_id: config.device_id.clone(),
        backend,
        stream: false,
        prompt: payload.prompt,
        model: model.clone(),
        mode: Some("local-first".to_string()),
        system_prompt: payload.system_prompt,
        max_tokens: payload.max_tokens,
        temperature: payload.temperature,
        top_p: payload.top_p,
        seed: payload.seed,
    };
    let result = worker::launch_worker(&worker_request, &config.effective_model_dir());
    let _ = lease_stop.send(());
    let _ = lease_handle.join();
    match result {
        Ok(response) => {
            let status = if response.status == "completed" {
                200
            } else {
                502
            };
            let body = LocalInferenceResponse {
                request_id,
                routing: if offline {
                    "local-offline".to_string()
                } else {
                    "local".to_string()
                },
                node_id: config.device_id.clone(),
                model: response.model.or(model),
                output: response.output,
                status: response.status,
                error: response.error,
            };
            let _ = request.respond(json_response(
                status,
                serde_json::to_value(body).expect("local response"),
            ));
        }
        Err(error) => {
            let _ = request.respond(json_response(
                502,
                serde_json::json!({ "error": error, "fallback": "network" }),
            ));
        }
    }
}

fn handle_request(
    request: Request,
    token: &str,
    config: &AgentConfig,
    identity: &DeviceIdentity,
    backend: Backend,
    slots: &Arc<SlotPool>,
) {
    if bearer_token(&request) != Some(token) {
        let _ = request.respond(json_response(
            401,
            serde_json::json!({ "error": "unauthorized" }),
        ));
        return;
    }
    match (request.method(), request.url()) {
        (&Method::Get, "/local/v1/health") => {
            let _ = request.respond(json_response(
                200,
                serde_json::json!({
                    "status": "ok",
                    "node_id": config.device_id,
                    "model": config.active_model,
                }),
            ));
        }
        (&Method::Get, "/local/v1/capacity") => {
            let _ = request.respond(json_response(
                200,
                serde_json::json!({
                    "total_slots": slots.capacity(),
                    "active_slots": slots.active(),
                    "available_slots": slots.available(),
                    "model": config.active_model,
                }),
            ));
        }
        (&Method::Post, "/local/v1/chat/completions") => {
            handle_local_inference(request, config, identity, backend, slots);
        }
        _ => {
            let _ = request.respond(json_response(
                404,
                serde_json::json!({ "error": "not found" }),
            ));
        }
    }
}

pub fn start(
    config: AgentConfig,
    identity: DeviceIdentity,
    backend: Backend,
    slots: Arc<SlotPool>,
) -> Result<LocalApiHandle, String> {
    let token = load_or_create_token()?;
    let address = local_api_addr();
    let server = Server::http(&address).map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let handle = thread::spawn(move || {
        while !thread_stop.load(Ordering::Acquire) {
            match server.recv_timeout(Duration::from_millis(250)) {
                Ok(Some(request)) => {
                    let config = config.clone();
                    let identity = identity.clone();
                    let slots = slots.clone();
                    let token = token.clone();
                    thread::spawn(move || {
                        handle_request(request, &token, &config, &identity, backend, &slots)
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("localApi: {error}");
                    break;
                }
            }
        }
    });
    Ok(LocalApiHandle {
        stop,
        handle: Some(handle),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        control_plane_is_unreachable, enabled, local_suitability_error,
        parameter_count_from_model_name, SlotPool,
    };
    use crate::storage::AgentConfig;

    #[test]
    fn slot_pool_never_oversubscribes() {
        let pool = SlotPool::new(2);
        let first = pool.try_acquire().expect("first slot");
        let second = pool.try_acquire().expect("second slot");
        assert!(pool.try_acquire().is_none());
        assert_eq!(pool.active(), 2);
        drop(first);
        assert!(pool.try_acquire().is_some());
        drop(second);
    }

    #[test]
    fn local_first_is_enabled_by_default_and_can_be_disabled() {
        let previous = std::env::var_os("OPENGPU_LOCAL_FIRST_ENABLED");
        std::env::remove_var("OPENGPU_LOCAL_FIRST_ENABLED");
        assert!(enabled());
        std::env::set_var("OPENGPU_LOCAL_FIRST_ENABLED", "false");
        assert!(!enabled());
        match previous {
            Some(value) => std::env::set_var("OPENGPU_LOCAL_FIRST_ENABLED", value),
            None => std::env::remove_var("OPENGPU_LOCAL_FIRST_ENABLED"),
        }
    }

    #[test]
    fn parses_parameter_count_from_common_model_names() {
        assert_eq!(
            parameter_count_from_model_name("Qwen/Qwen2.5-3B-Instruct"),
            Some(3_000_000_000)
        );
        assert_eq!(
            parameter_count_from_model_name("llama-3.2-1.5b"),
            Some(1_500_000_000)
        );
    }

    #[test]
    fn small_models_accept_light_work_but_defer_complex_work() {
        let mut config = AgentConfig::default();
        config.active_model = Some("Qwen/Qwen2.5-3B-Instruct".to_string());
        assert!(local_suitability_error(
            &config,
            config.active_model.as_deref(),
            "Translate this sentence to English",
            Some(256),
        )
        .is_none());
        let error = local_suitability_error(
            &config,
            config.active_model.as_deref(),
            "Create a complete production-ready CRUD API with tests and architecture",
            Some(2_048),
        )
        .expect("small model should defer complex work");
        assert!(error.contains("control-plane planner"));
    }

    #[test]
    fn offline_mode_accepts_only_transport_failures() {
        assert!(control_plane_is_unreachable(
            "transport failed: connection refused"
        ));
        assert!(!control_plane_is_unreachable("HTTP 409: lease denied"));
        assert!(!control_plane_is_unreachable(
            "control-plane-url must start with http:// or https://"
        ));
    }
}
