mod contracts;
mod state;
mod supabase;

use contracts::{AgentRegistration, Heartbeat, JobCompletion, JobRequest};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use state::{load_state, save_state, ControlPlaneState};
use supabase::SupabaseMirror;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn json_response(status: &str, body: serde_json::Value) -> String {
    let payload = body.to_string();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    )
}

fn text_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn html_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn load_local_env() {
    let mut current = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(_) => return,
    };

    loop {
        let env_path = current.join(".env");
        if env_path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&env_path) {
                for line in raw.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    let Some((key, value)) = trimmed.split_once('=') else {
                        continue;
                    };
                    let key = key.trim();
                    let value = value.trim().trim_matches('"');
                    if !key.is_empty() && std::env::var_os(key).is_none() {
                        std::env::set_var(key, value);
                    }
                }
            }
            return;
        }

        if !current.pop() {
            return;
        }
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn state_badge(state: &str) -> (&'static str, &'static str) {
    match state {
        "ready" => ("#12351f", "#8ef0aa"),
        "busy" => ("#3d2b0f", "#ffd27f"),
        "paused" => ("#3a2610", "#ffbf7a"),
        "stopped" => ("#3b1515", "#ff9d9d"),
        _ => ("#22304c", "#b8c7e8"),
    }
}

fn policy_badge(allowed: bool) -> (&'static str, &'static str, &'static str) {
    if allowed {
        ("#12351f", "#8ef0aa", "allowed")
    } else {
        ("#3b1515", "#ff9d9d", "blocked")
    }
}

fn render_nodes(state: &ControlPlaneState) -> String {
    let nodes: Vec<_> = state.nodes.values().cloned().collect();
    if nodes.is_empty() {
        return r#"<div class="empty">No nodes have registered yet.</div>"#.to_string();
    }

    let mut html = String::from(
        r#"<div class="table">
        <div class="thead">
          <div>Node</div>
          <div>Host</div>
          <div>Backend</div>
          <div>State</div>
          <div>Power</div>
          <div>Policy</div>
          <div>Updated</div>
        </div>"#,
    );

    for node in nodes {
        let (state_bg, state_fg) = state_badge(node.state.as_str());
        let (policy_bg, policy_fg, policy_label) = policy_badge(node.policy_allowed);
        let battery = node
            .battery_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "unknown".to_string());
        let power = format!(
            "{} • {} • {}",
            node.power_source,
            if node.on_battery { "battery" } else { "AC" },
            battery
        );
        let policy_reason = node
            .policy_reason
            .as_ref()
            .map(|reason| format!(r#"<div class="meta">{}</div>"#, escape_html(reason)))
            .unwrap_or_default();

        html.push_str(&format!(
            r#"<div class="row">
              <div>
                <strong>{}</strong>
                <div class="meta">fingerprint {}</div>
                <div class="meta">cap {}% • {} GPU% free</div>
              </div>
              <div>
                <div>{}</div>
                <div class="meta">signed device</div>
              </div>
              <div><span class="pill" style="background:{};color:{};">{}</span></div>
              <div>
                <span class="pill" style="background:{};color:{};">{}</span>
                <div class="meta" style="margin-top:6px;">{}</div>
              </div>
              <div>
                <div>{}</div>
                <div class="meta">{}</div>
              </div>
              <div>
                <span class="pill" style="background:{};color:{};">{}</span>
                {}
              </div>
              <div>{}</div>
            </div>"#,
            escape_html(&node.node_id),
            escape_html(&node.public_key_fingerprint),
            node.contribution_percent,
            node.available_gpu_percent,
            escape_html(&node.hostname),
            escape_html(&node.backend.to_string()),
            state_bg,
            state_fg,
            escape_html(&node.state.to_string()),
            state_bg,
            state_fg,
            escape_html(&node.state.to_string()),
            escape_html(&power),
            escape_html(&node.public_key_fingerprint),
            policy_bg,
            policy_fg,
            policy_label,
            policy_reason,
            escape_html(&node.updated_at)
        ));
    }

    html.push_str("</div>");
    html
}

fn control_plane_home(state: &ControlPlaneState) -> String {
    let snapshot = state.snapshot();
    let nodes = snapshot["online_count"].as_u64().unwrap_or(0);
    let paused = snapshot["paused_count"].as_u64().unwrap_or(0);
    let policy_blocked = snapshot["policy_blocked_count"].as_u64().unwrap_or(0);
    let queued = snapshot["queued_job_count"].as_u64().unwrap_or(0);
    let assigned = snapshot["assigned_job_count"].as_u64().unwrap_or(0);
    let completed = snapshot["completed_job_count"].as_u64().unwrap_or(0);
    let failed = snapshot["failed_job_count"].as_u64().unwrap_or(0);

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>OpenGPU Control Plane</title>
    <style>
      body {{
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        background: #0b1220;
        color: #e5eefc;
        margin: 0;
        padding: 32px;
      }}
      .card {{
        max-width: 900px;
        margin: 0 auto;
        background: #111a2e;
        border: 1px solid #24324f;
        border-radius: 20px;
        padding: 28px;
        box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
      }}
      .badge {{
        display: inline-block;
        padding: 6px 10px;
        border-radius: 999px;
        background: #12351f;
        color: #8ef0aa;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
      }}
      h1 {{
        margin: 16px 0 8px;
        font-size: 34px;
      }}
      p {{
        color: #a8b4cb;
        line-height: 1.6;
      }}
      .grid {{
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
        gap: 14px;
        margin: 24px 0;
      }}
      .stat {{
        background: #0d1526;
        border: 1px solid #23314e;
        border-radius: 16px;
        padding: 16px;
      }}
      .stat span {{
        display: block;
        font-size: 12px;
        color: #8da0c4;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }}
      .stat strong {{
        display: block;
        margin-top: 6px;
        font-size: 28px;
        color: #ffffff;
      }}
      .section-title {{
        margin: 30px 0 10px;
        font-size: 18px;
        color: #f1f6ff;
      }}
      .table {{
        display: grid;
        gap: 10px;
        margin-top: 18px;
      }}
      .thead,
      .row {{
        display: grid;
        grid-template-columns: 1.4fr 1fr 0.9fr 0.8fr 1.4fr 1fr 0.7fr;
        gap: 12px;
        align-items: start;
      }}
      .thead {{
        color: #91a6cb;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 11px;
        padding: 0 12px;
      }}
      .row {{
        background: #0d1526;
        border: 1px solid #23314e;
        border-radius: 16px;
        padding: 14px 12px;
      }}
      .row strong {{
        display: block;
        font-size: 14px;
        color: #f4f8ff;
        margin-bottom: 4px;
      }}
      .meta {{
        color: #8ea4ca;
        font-size: 12px;
        line-height: 1.45;
      }}
      .pill {{
        display: inline-block;
        padding: 4px 9px;
        border-radius: 999px;
        font-size: 11px;
        text-transform: uppercase;
        letter-spacing: 0.07em;
      }}
      .note {{
        margin-top: 16px;
        padding: 14px 16px;
        border-radius: 14px;
        background: #0a1020;
        border: 1px solid #24324f;
        color: #b6c5e4;
      }}
      .empty {{
        margin-top: 18px;
        padding: 18px;
        border-radius: 16px;
        border: 1px dashed #24324f;
        background: #0a1020;
        color: #8ea4ca;
      }}
      code {{
        background: #0a1020;
        border: 1px solid #24324f;
        padding: 2px 6px;
        border-radius: 8px;
        color: #9fe7b0;
      }}
      a {{
        color: #9fe7b0;
      }}
    </style>
  </head>
  <body>
    <main class="card">
      <span class="badge">healthy</span>
      <h1>OpenGPU control plane</h1>
      <p>This is the local prototype registry and job queue running at <code>http://127.0.0.1:8787</code>.</p>
      <div class="grid">
        <div class="stat"><span>Online nodes</span><strong>{nodes}</strong></div>
        <div class="stat"><span>Paused nodes</span><strong>{paused}</strong></div>
        <div class="stat"><span>Policy blocked</span><strong>{policy_blocked}</strong></div>
        <div class="stat"><span>Queued jobs</span><strong>{queued}</strong></div>
        <div class="stat"><span>Assigned jobs</span><strong>{assigned}</strong></div>
        <div class="stat"><span>Completed jobs</span><strong>{completed}</strong></div>
        <div class="stat"><span>Failed jobs</span><strong>{failed}</strong></div>
      </div>
      <div class="note">
        Policy-aware nodes are still visible in the registry, but nodes that should stay quiet are excluded from scheduling.
      </div>
      <h2 class="section-title">Node details</h2>
      {node_rows}
      <p>Useful endpoints: <a href="/health">/health</a>, <a href="/v1/status">/v1/status</a>, <a href="/v1/nodes">/v1/nodes</a>, <a href="/v1/jobs">/v1/jobs</a></p>
    </main>
  </body>
</html>"#
        ,
        node_rows = render_nodes(state)
    )
}

struct RequestParts {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

fn parse_request(request: &str) -> RequestParts {
    let mut lines = request.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = BTreeMap::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
    RequestParts {
        method,
        path,
        headers,
        body,
    }
}

fn split_path_and_query(path: &str) -> (&str, Option<&str>) {
    if let Some((clean_path, query)) = path.split_once('?') {
        (clean_path, Some(query))
    } else {
        (path, None)
    }
}

fn query_param<'a>(query: Option<&'a str>, key: &str) -> Option<&'a str> {
    let query = query?;
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let candidate_key = parts.next().unwrap_or_default();
        let candidate_value = parts.next().unwrap_or_default();
        if candidate_key == key {
            return Some(candidate_value);
        }
    }
    None
}

fn header_value<'a>(headers: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    headers.get(&key.to_ascii_lowercase()).map(|value| value.as_str())
}

fn requires_device_signature(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        ("POST", "/v1/register")
            | ("POST", "/v1/heartbeat")
            | ("GET", "/v1/jobs/next")
            | ("POST", "/v1/jobs/complete")
    )
}

fn node_public_key_hex(state: &Arc<Mutex<ControlPlaneState>>, node_id: &str) -> Option<String> {
    state
        .lock()
        .expect("state lock")
        .nodes
        .get(node_id)
        .map(|node| node.public_key_hex.clone())
}

fn verify_signature(
    public_key_hex: &str,
    method: &str,
    path: &str,
    timestamp: &str,
    body: &str,
    signature_hex: &str,
) -> Result<(), String> {
    let public_bytes = hex::decode(public_key_hex).map_err(|error| error.to_string())?;
    let public_bytes: [u8; 32] = public_bytes
        .try_into()
        .map_err(|_| "public key must be 32 bytes".to_string())?;
    let verifying_key = VerifyingKey::from_bytes(&public_bytes).map_err(|error| error.to_string())?;
    let signature_bytes = hex::decode(signature_hex).map_err(|error| error.to_string())?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|error| error.to_string())?;
    let message = format!("{method}\n{path}\n{timestamp}\n{body}");
    verifying_key
        .verify(message.as_bytes(), &signature)
        .map_err(|error| error.to_string())
}

fn authorize_device_request(
    method: &str,
    route_path: &str,
    request_path: &str,
    headers: &BTreeMap<String, String>,
    body: &str,
    state: &Arc<Mutex<ControlPlaneState>>,
) -> Result<(), String> {
    if !requires_device_signature(method, route_path) {
        return Ok(());
    }

    let node_id = header_value(headers, "x-opengpu-node-id")
        .ok_or_else(|| "missing x-opengpu-node-id".to_string())?;
    let timestamp = header_value(headers, "x-opengpu-timestamp")
        .ok_or_else(|| "missing x-opengpu-timestamp".to_string())?;
    let signature = header_value(headers, "x-opengpu-signature")
        .ok_or_else(|| "missing x-opengpu-signature".to_string())?;

    let timestamp_value = timestamp
        .parse::<i64>()
        .map_err(|_| "invalid x-opengpu-timestamp".to_string())?;
    let current = now_unix_seconds()
        .parse::<i64>()
        .map_err(|_| "invalid current timestamp".to_string())?;
    if current.abs_diff(timestamp_value) > 300 {
        return Err("signature timestamp expired".to_string());
    }

    let public_key_hex = if route_path == "/v1/register" {
        let registration: AgentRegistration =
            serde_json::from_str(body).map_err(|error| error.to_string())?;
        if registration.node_id != node_id {
            return Err("node id header mismatch".to_string());
        }
        registration.public_key_hex
    } else {
        node_public_key_hex(state, node_id).ok_or_else(|| "unknown node".to_string())?
    };

    verify_signature(&public_key_hex, method, request_path, timestamp, body, signature)
}

fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<ControlPlaneState>>,
    supabase: Option<&SupabaseMirror>,
) {
    let mut buffer = vec![0; 16 * 1024];
    let read_result = stream.read(&mut buffer);
    let bytes_read = match read_result {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = stream.write_all(text_response("400 Bad Request", &error.to_string()).as_bytes());
            return;
        }
    };

    let request_text = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
    let request = parse_request(&request_text);
    let (clean_path, query) = split_path_and_query(&request.path);

    if let Err(error) = authorize_device_request(
        &request.method,
        clean_path,
        &request.path,
        &request.headers,
        &request.body,
        &state,
    ) {
        let _ = stream.write_all(
            json_response("401 Unauthorized", serde_json::json!({ "error": error })).as_bytes(),
        );
        return;
    }

    let response = match (request.method.as_str(), clean_path) {
        ("GET", "/") => {
            let snapshot = state.lock().expect("state lock");
            html_response("200 OK", &control_plane_home(&snapshot))
        }
        ("GET", "/health") => text_response("200 OK", "ok"),
        ("GET", "/v1/status") => {
            let snapshot = state.lock().expect("state lock").snapshot();
            json_response("200 OK", snapshot)
        }
        ("GET", "/v1/nodes") => {
            let snapshot = state.lock().expect("state lock").nodes_snapshot();
            json_response("200 OK", snapshot)
        }
        ("GET", "/v1/jobs") => {
            let snapshot = state.lock().expect("state lock").jobs_snapshot();
            json_response("200 OK", snapshot)
        }
        ("GET", "/v1/jobs/next") => {
            if let Some(node_id) = query_param(query, "node_id") {
                let mut guard = state.lock().expect("state lock");
                let claim = guard.claim_job(node_id, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Some(job) = claim.job.as_ref() {
                        if let Err(error) = db.record_job_event(
                            Some(node_id),
                            Some(&job.job_id),
                            "job_claimed",
                            serde_json::to_value(job).expect("json"),
                        ) {
                            eprintln!("database claim sync skipped: {error}");
                        }
                    }
                }
                json_response("200 OK", serde_json::to_value(claim).expect("json"))
            } else {
                text_response("400 Bad Request", "missing node_id")
            }
        }
        ("POST", "/v1/register") => match serde_json::from_str::<AgentRegistration>(&request.body) {
            Ok(registration) => {
                let registration_clone = registration.clone();
                let mut guard = state.lock().expect("state lock");
                let record = guard.register(registration);
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Err(error) = db.record_registration(&registration_clone) {
                        eprintln!("database registration sync skipped: {error}");
                    }
                    if let Err(error) = db.record_job_event(
                        Some(&record.node_id),
                        None,
                        "registration",
                        serde_json::to_value(&record).expect("json"),
                    ) {
                        eprintln!("database registration event skipped: {error}");
                    }
                }
                json_response("200 OK", serde_json::to_value(record).expect("json"))
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        ("POST", "/v1/heartbeat") => match serde_json::from_str::<Heartbeat>(&request.body) {
            Ok(heartbeat) => {
                let heartbeat_clone = heartbeat.clone();
                let mut guard = state.lock().expect("state lock");
                let record = guard.heartbeat(heartbeat, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Err(error) = db.record_heartbeat(&heartbeat_clone) {
                        eprintln!("database heartbeat sync skipped: {error}");
                    }
                    if let Err(error) = db.record_job_event(
                        Some(&record.node_id),
                        None,
                        "heartbeat",
                        serde_json::to_value(&record).expect("json"),
                    ) {
                        eprintln!("database heartbeat event skipped: {error}");
                    }
                }
                json_response("200 OK", serde_json::to_value(record).expect("json"))
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        ("POST", "/v1/jobs") => match serde_json::from_str::<JobRequest>(&request.body) {
            Ok(request) => {
                let mut guard = state.lock().expect("state lock");
                let record = guard.submit_job(request, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Err(error) = db.record_job(&record) {
                        eprintln!("database job sync skipped: {error}");
                    }
                    if let Err(error) = db.record_job_event(
                        None,
                        Some(&record.job_id),
                        "job_submitted",
                        serde_json::to_value(&record).expect("json"),
                    ) {
                        eprintln!("database job event skipped: {error}");
                    }
                }
                json_response("200 OK", serde_json::to_value(record).expect("json"))
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        ("POST", "/v1/jobs/complete") => match serde_json::from_str::<JobCompletion>(&request.body) {
            Ok(completion) => {
                let completion_clone = completion.clone();
                let mut guard = state.lock().expect("state lock");
                let record = guard.complete_job(completion, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Some(job) = record.as_ref() {
                        if let Err(error) = db.record_job_completion(&completion_clone, job) {
                            eprintln!("database completion sync skipped: {error}");
                        }
                    }
                }
                match record {
                    Some(record) => {
                        json_response("200 OK", serde_json::to_value(record).expect("json"))
                    }
                    None => json_response(
                        "404 Not Found",
                        serde_json::json!({ "error": "job not found" }),
                    ),
                }
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        _ => text_response("404 Not Found", "not found"),
    };

    let _ = stream.write_all(response.as_bytes());
}

fn main() {
    load_local_env();
    let supabase = SupabaseMirror::from_env();
    let listener = TcpListener::bind("127.0.0.1:8787").expect("bind control plane");
    let state = Arc::new(Mutex::new(load_state().ok().flatten().unwrap_or_default()));

    println!("OpenGPU control plane listening on http://127.0.0.1:8787");
    println!("supabase: {}", SupabaseMirror::startup_status());
    println!("home: GET /");
    println!("health: GET /health");
    println!("status: GET /v1/status");
    println!("nodes: GET /v1/nodes");
    println!("jobs: GET /v1/jobs");
    println!("register: POST /v1/register");
    println!("heartbeat: POST /v1/heartbeat");
    println!("submit job: POST /v1/jobs");
    println!("claim job: GET /v1/jobs/next?node_id=...");
    println!("complete job: POST /v1/jobs/complete");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let state = Arc::clone(&state);
                handle_connection(stream, state, supabase.as_ref());
            }
            Err(error) => eprintln!("incoming connection error: {error}"),
        }
    }
}
