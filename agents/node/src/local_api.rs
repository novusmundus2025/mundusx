use crate::contracts::{
    Backend, LocalSlotLeaseReleaseRequest, LocalSlotLeaseRenewRequest, LocalSlotLeaseRequest,
    LocalSlotLeaseResponse, WorkerLaunchRequest,
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
}

#[derive(Debug, Serialize)]
struct LocalInferenceResponse {
    request_id: String,
    routing: &'static str,
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

fn start_lease_renewal(
    config: AgentConfig,
    identity: DeviceIdentity,
    lease_id: String,
) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::spawn(move || loop {
        match stop_rx.recv_timeout(Duration::from_secs(LOCAL_LEASE_RENEW_SECONDS)) {
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let payload = LocalSlotLeaseRenewRequest {
                    lease_id: lease_id.clone(),
                    ttl_seconds: LOCAL_LEASE_TTL_SECONDS,
                };
                if let Err(error) = signed_post_json_body::<_, LocalSlotLeaseResponse>(
                    &config.control_plane_url,
                    "/v1/local-leases/renew",
                    &config.device_id,
                    &identity,
                    &payload,
                ) {
                    eprintln!("localLeaseRenewal: {error}");
                }
            }
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
    let lease = match signed_post_json_body::<_, LocalSlotLeaseResponse>(
        &config.control_plane_url,
        "/v1/local-leases",
        &config.device_id,
        identity,
        &LocalSlotLeaseRequest {
            request_id: request_id.clone(),
            slots: 1,
            model: model.clone(),
            ttl_seconds: LOCAL_LEASE_TTL_SECONDS,
        },
    ) {
        Ok(response) if response.granted => response.lease,
        Ok(response) => {
            let _ = request.respond(json_response(
                409,
                serde_json::json!({ "error": response.error.unwrap_or_else(|| "local lease denied".to_string()), "fallback": "network" }),
            ));
            return;
        }
        Err(error) => {
            let _ = request.respond(json_response(
                503,
                serde_json::json!({ "error": format!("control-plane lease unavailable: {error}"), "fallback": "network" }),
            ));
            return;
        }
    };
    let Some(lease) = lease else {
        let _ = request.respond(json_response(
            503,
            serde_json::json!({ "error": "control plane returned no lease" }),
        ));
        return;
    };
    let (renew_stop, renew_handle) =
        start_lease_renewal(config.clone(), identity.clone(), lease.lease_id.clone());
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
    let _ = renew_stop.send(());
    let _ = renew_handle.join();
    let _ = signed_post_json_body::<_, LocalSlotLeaseResponse>(
        &config.control_plane_url,
        "/v1/local-leases/release",
        &config.device_id,
        identity,
        &LocalSlotLeaseReleaseRequest {
            lease_id: lease.lease_id,
        },
    );
    match result {
        Ok(response) => {
            let status = if response.status == "completed" {
                200
            } else {
                502
            };
            let body = LocalInferenceResponse {
                request_id,
                routing: "local",
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
    use super::{enabled, SlotPool};

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
}
