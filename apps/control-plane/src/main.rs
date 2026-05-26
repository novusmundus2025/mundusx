mod contracts;
mod migrations;
mod state;
mod supabase;

use contracts::{
    AgentRegistration, ChatCompletionChoice, ChatCompletionChoiceMessage, ChatCompletionOpenGpu,
    ChatCompletionRequest, ChatCompletionResponse, Heartbeat, JobCompletion, JobRequest,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use migrations::{applied_migrations, apply_migrations};
use state::{load_state, save_state, state_path, ControlPlaneState};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use supabase::SupabaseMirror;
use uuid::Uuid;

#[cfg(target_os = "macos")]
#[path = "../../../tools/macos_identity.rs"]
mod macos_identity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StorageSource {
    Supabase,
    LocalJsonFallback,
    LocalJsonOnly,
}

impl StorageSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Supabase => "supabase",
            Self::LocalJsonFallback => "local-json-fallback",
            Self::LocalJsonOnly => "local-json-only",
        }
    }
}

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

fn chat_messages_to_prompt(messages: &[contracts::ChatMessage]) -> (Option<String>, String) {
    let mut system_messages = Vec::new();
    let mut conversation_lines = Vec::new();

    for message in messages {
        let role = message.role.trim().to_lowercase();
        let content = message.content.trim();
        if content.is_empty() {
            continue;
        }

        if role == "system" {
            system_messages.push(content.to_string());
        } else {
            conversation_lines.push(format!("{role}: {content}"));
        }
    }

    let system_prompt = if system_messages.is_empty() {
        None
    } else {
        Some(system_messages.join("\n"))
    };

    let prompt = if conversation_lines.is_empty() {
        messages
            .last()
            .map(|message| message.content.clone())
            .unwrap_or_default()
    } else {
        conversation_lines.join("\n")
    };

    (system_prompt, prompt)
}

fn now_unix_seconds_u64() -> u64 {
    now_unix_seconds().parse::<u64>().unwrap_or(0)
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
          <div>Trust</div>
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
        let worker_health = node
            .worker_health
            .as_ref()
            .map(|health| {
                let model_name = health.model_name.as_deref().unwrap_or("none");
                let model_path = health.model_path.as_deref().unwrap_or("missing");
                let notes = if health.notes.is_empty() {
                    "no notes".to_string()
                } else {
                    health.notes.join(" • ")
                };
                format!(
                    r#"<div class="meta">worker: {} • model {} • {} • llama-cli {} • BLAS {}</div><div class="meta">{}</div>"#,
                    if health.healthy { "healthy" } else { "degraded" },
                    escape_html(model_name),
                    escape_html(model_path),
                    if health.llama_cli_available { "yes" } else { "no" },
                    if health.blas_device_available { "yes" } else { "no" },
                    escape_html(&notes)
                )
            })
            .unwrap_or_else(|| r#"<div class="meta">worker: unknown</div>"#.to_string());
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
              <div>
                <div>{}</div>
                <div class="meta">identity path</div>
              </div>
              <div><span class="pill" style="background:{};color:{};">{}</span></div>
              <div>
                <span class="pill" style="background:{};color:{};">{}</span>
                <div class="meta" style="margin-top:6px;">{}</div>
              </div>
              <div>
                <div>{}</div>
                <div class="meta">{}</div>
                {}
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
            escape_html(&node.identity_trust_path),
            escape_html(&node.backend.to_string()),
            state_bg,
            state_fg,
            escape_html(&node.state.to_string()),
            state_bg,
            state_fg,
            escape_html(&node.state.to_string()),
            escape_html(&power),
            escape_html(&node.public_key_fingerprint),
            worker_health,
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

fn control_plane_home(state: &ControlPlaneState, storage_source: StorageSource) -> String {
    let snapshot = state.snapshot(storage_source.as_str());
    let nodes = snapshot["online_count"].as_u64().unwrap_or(0);
    let paused = snapshot["paused_count"].as_u64().unwrap_or(0);
    let policy_blocked = snapshot["policy_blocked_count"].as_u64().unwrap_or(0);
    let job_events = snapshot["job_events"].as_u64().unwrap_or(0);
    let credits_ledger = snapshot["credits_ledger"].as_u64().unwrap_or(0);
    let credits_total = snapshot["credits_total"].as_f64().unwrap_or(0.0);
    let queued = snapshot["queued_job_count"].as_u64().unwrap_or(0);
    let assigned = snapshot["assigned_job_count"].as_u64().unwrap_or(0);
    let completed = snapshot["completed_job_count"].as_u64().unwrap_or(0);
    let failed = snapshot["failed_job_count"].as_u64().unwrap_or(0);
    let healthy_tone = "green";
    let storage_tone = if storage_source.as_str() == "supabase" {
        "green"
    } else {
        "amber"
    };
    let supabase = SupabaseMirror::startup_status();
    let supabase_tone = if supabase.as_str().starts_with("enabled") {
        "green"
    } else {
        "red"
    };

    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>NovusX Control Plane</title>
    <style>
      :root {{
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --line-strong: rgba(15, 23, 42, 0.14);
        --text: #0f172a;
        --muted: #5f6b85;
        --blue: #3452ff;
        --green: #0f9d58;
        --orange: #c47f1b;
        --amber: #d97706;
        --red: #d14343;
      }}
      * {{ box-sizing: border-box; }}
      body {{
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 30%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
      }}
      .wrap {{
        max-width: 1380px;
        margin: 0 auto;
        padding: 22px 20px 48px;
      }}
      .hero {{
        border: 1px solid var(--line);
        background: rgba(255, 255, 255, 0.92);
        border-radius: 22px;
        padding: 24px;
        box-shadow: 0 18px 60px rgba(15, 23, 42, 0.06);
      }}
      .topline {{
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 20px;
        flex-wrap: wrap;
      }}
      .brand {{
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 800;
        letter-spacing: 0.02em;
      }}
      .brand-mark {{
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }}
      h1 {{
        margin: 0;
        font-size: clamp(40px, 5vw, 64px);
        line-height: 0.96;
        letter-spacing: -0.06em;
      }}
      .sub {{
        margin-top: 12px;
        color: var(--muted);
        line-height: 1.7;
      }}
      .statusline {{
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-top: 18px;
      }}
      .pill {{
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid transparent;
      }}
      .pill-green {{ background: rgba(15, 157, 88, 0.08); color: var(--green); border-color: rgba(15, 157, 88, 0.16); }}
      .pill-orange {{ background: rgba(196, 127, 27, 0.08); color: var(--orange); border-color: rgba(196, 127, 27, 0.16); }}
      .pill-amber {{ background: rgba(217, 119, 6, 0.08); color: var(--amber); border-color: rgba(217, 119, 6, 0.16); }}
      .pill-red {{ background: rgba(209, 67, 67, 0.08); color: var(--red); border-color: rgba(209, 67, 67, 0.16); }}
      .pill-blue {{ background: rgba(52, 82, 255, 0.08); color: var(--blue); border-color: rgba(52, 82, 255, 0.16); }}
      .pill-neutral {{ background: rgba(95, 107, 133, 0.08); color: var(--muted); border-color: rgba(95, 107, 133, 0.16); }}
      .grid {{
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: 14px;
        margin: 18px 0 24px;
      }}
      .card {{
        border: 1px solid var(--line);
        background: var(--surface);
        border-radius: 18px;
        padding: 16px;
      }}
      .card-label {{
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }}
      .card-value {{
        margin: 10px 0 12px;
        font-size: 30px;
        font-weight: 700;
      }}
      .section {{
        margin-top: 24px;
        border: 1px solid var(--line);
        background: rgba(255, 255, 255, 0.92);
        border-radius: 22px;
        overflow: hidden;
        box-shadow: 0 18px 60px rgba(15, 23, 42, 0.05);
      }}
      .section-head {{
        padding: 16px 20px;
        border-bottom: 1px solid var(--line);
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
      }}
      .section-title {{
        margin: 0;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 14px;
      }}
      .section-body {{
        padding: 20px;
      }}
      .table .thead,
      .table .row {{
        display: grid;
        grid-template-columns: 1.3fr 1fr 0.7fr 0.7fr 1fr 1.1fr 0.7fr;
        gap: 14px;
        align-items: start;
      }}
      .table .thead {{
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
        padding-bottom: 12px;
        margin-bottom: 12px;
        border-bottom: 1px solid var(--line);
      }}
      .table .row {{
        padding: 14px 0;
        border-bottom: 1px solid rgba(15, 23, 42, 0.06);
      }}
      .table .row:last-child {{ border-bottom: 0; }}
      .meta {{
        color: var(--muted);
        font-size: 12px;
        line-height: 1.4;
      }}
      .empty {{
        color: var(--muted);
        padding: 28px 0;
      }}
      .events {{
        display: grid;
        gap: 12px;
      }}
      .balance-grid {{
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
        gap: 12px;
        margin-bottom: 18px;
      }}
      .balance {{
        border: 1px solid var(--line);
        border-radius: 14px;
        background: var(--surface);
        padding: 14px 16px;
      }}
      .balance strong {{
        display: block;
        margin-bottom: 6px;
        font-size: 14px;
        color: var(--text);
        overflow-wrap: anywhere;
      }}
      .event {{
        border: 1px solid var(--line);
        border-radius: 14px;
        background: var(--surface);
        padding: 14px 16px;
      }}
      .event-top {{
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
      }}
      pre {{
        overflow: auto;
        margin: 12px 0 0;
        color: #31415f;
        font-size: 12px;
        line-height: 1.5;
        white-space: pre-wrap;
        word-break: break-word;
      }}
      .error {{
        margin-top: 18px;
        border: 1px solid rgba(209, 67, 67, 0.22);
        background: rgba(209, 67, 67, 0.06);
        color: var(--red);
        padding: 14px 16px;
        border-radius: 14px;
      }}
      .links {{
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
      }}
      a {{
        color: var(--blue);
        text-decoration: none;
      }}
      a:hover {{ text-decoration: underline; }}
      code {{
        background: rgba(52, 82, 255, 0.06);
        border: 1px solid rgba(52, 82, 255, 0.1);
        padding: 2px 6px;
        border-radius: 8px;
        color: var(--text);
      }}
      @media (max-width: 1200px) {{
        .grid {{ grid-template-columns: repeat(2, minmax(0, 1fr)); }}
        .table .thead,
        .table .row {{ grid-template-columns: 1.1fr 0.9fr 0.7fr 0.7fr 1fr 1fr 0.7fr; }}
      }}
      @media (max-width: 820px) {{
        .grid {{ grid-template-columns: 1fr; }}
        .table .thead {{ display: none; }}
        .table .row {{
          grid-template-columns: 1fr;
          gap: 10px;
          padding: 16px 0;
        }}
      }}
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="hero">
        <div class="topline">
            <div>
            <div class="brand"><span class="brand-mark"></span> NovusX Control Plane</div>
            <div class="sub">Local operator view for nodes, jobs, storage source, and audit trail.</div>
            <div class="statusline">
              <span class="pill pill-{healthy_tone}">healthy</span>
              <span class="pill pill-{storage_tone}">storage: {storage_source}</span>
              <span class="pill pill-{supabase_tone}">supabase: {supabase}</span>
            </div>
          </div>
          <div class="links">
            <a href="/health">health</a>
            <a href="/v1/status">status json</a>
            <a href="/v1/nodes">nodes json</a>
            <a href="/v1/jobs">jobs json</a>
            <a href="/v1/credits">credits json</a>
          </div>
        </div>

        <div class="grid">
          <div class="card"><div class="card-label">Online nodes</div><div class="card-value">{nodes}</div></div>
          <div class="card"><div class="card-label">Paused nodes</div><div class="card-value">{paused}</div></div>
          <div class="card"><div class="card-label">Policy blocked</div><div class="card-value">{policy_blocked}</div></div>
          <div class="card"><div class="card-label">Job events</div><div class="card-value">{job_events}</div></div>
          <div class="card"><div class="card-label">Credits ledger</div><div class="card-value">{credits_ledger}</div></div>
          <div class="card"><div class="card-label">Total credits</div><div class="card-value">{credits_total:.2}</div></div>
          <div class="card"><div class="card-label">Queued jobs</div><div class="card-value">{queued}</div></div>
          <div class="card"><div class="card-label">Assigned jobs</div><div class="card-value">{assigned}</div></div>
          <div class="card"><div class="card-label">Completed jobs</div><div class="card-value">{completed}</div></div>
          <div class="card"><div class="card-label">Failed jobs</div><div class="card-value">{failed}</div></div>
        </div>

        <div class="error">
          Policy-aware nodes stay visible in the registry, but quiet nodes are excluded from scheduling.
          Current startup storage source: <code>{storage_source}</code>. Credits are accrued through the
          append-only ledger and exposed at <code>/v1/credits</code>.
        </div>
      </div>

      <div class="section">
        <div class="section-head">
          <h2 class="section-title">Node details</h2>
          <div class="meta">{nodes} registered</div>
        </div>
        <div class="section-body">
          {node_rows}
        </div>
      </div>
    </div>
  </body>
</html>"#,
        node_rows = render_nodes(state),
        storage_source = escape_html(storage_source.as_str())
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
    let body = request
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string();
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
    headers
        .get(&key.to_ascii_lowercase())
        .map(|value| value.as_str())
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

fn requires_operator_auth(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        ("GET", "/")
            | ("GET", "/v1/status")
            | ("GET", "/v1/nodes")
            | ("GET", "/v1/jobs")
            | ("GET", "/v1/job-events")
            | ("GET", "/v1/credits")
            | ("POST", "/v1/jobs")
            | ("POST", "/v1/chat/completions")
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
    let message = format!("{method}\n{path}\n{timestamp}\n{body}");

    #[cfg(target_os = "macos")]
    if public_bytes.len() == 65 {
        let storage_dir = state_path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".opengpu-control-plane"));
        let verified = macos_identity::verify_message(
            &storage_dir,
            public_key_hex,
            message.as_bytes(),
            signature_hex,
        )
        .map_err(|error| error.to_string())?;
        if verified {
            return Ok(());
        }
        return Err("signature verification failed".to_string());
    }

    let public_bytes: [u8; 32] = public_bytes.try_into().map_err(|_| {
        "public key must be 32 bytes or macOS secure-enclave public key".to_string()
    })?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_bytes).map_err(|error| error.to_string())?;
    let signature_bytes = hex::decode(signature_hex).map_err(|error| error.to_string())?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|error| error.to_string())?;
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

    verify_signature(
        &public_key_hex,
        method,
        request_path,
        timestamp,
        body,
        signature,
    )
}

fn operator_auth_token() -> Option<String> {
    std::env::var("OPENGPU_OPERATOR_TOKEN")
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn authorize_operator_request(
    method: &str,
    route_path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(), String> {
    if !requires_operator_auth(method, route_path) {
        return Ok(());
    }

    let Some(expected_token) = operator_auth_token() else {
        return Ok(());
    };

    let authorization = header_value(headers, "authorization")
        .or_else(|| header_value(headers, "x-opengpu-operator-token"))
        .ok_or_else(|| "missing operator authorization".to_string())?;

    let presented = authorization
        .strip_prefix("Bearer ")
        .or_else(|| authorization.strip_prefix("bearer "))
        .unwrap_or(authorization)
        .trim();

    if presented != expected_token {
        return Err("invalid operator token".to_string());
    }

    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    state: Arc<Mutex<ControlPlaneState>>,
    supabase: Option<&SupabaseMirror>,
    storage_source: StorageSource,
) {
    let mut buffer = vec![0; 16 * 1024];
    let read_result = stream.read(&mut buffer);
    let bytes_read = match read_result {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ =
                stream.write_all(text_response("400 Bad Request", &error.to_string()).as_bytes());
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

    if let Err(error) = authorize_operator_request(&request.method, clean_path, &request.headers) {
        let _ = stream.write_all(
            json_response("401 Unauthorized", serde_json::json!({ "error": error })).as_bytes(),
        );
        return;
    }

    let response = match (request.method.as_str(), clean_path) {
        ("GET", "/") => {
            let snapshot = state.lock().expect("state lock");
            html_response("200 OK", &control_plane_home(&snapshot, storage_source))
        }
        ("GET", "/health") => {
            let snapshot = state
                .lock()
                .expect("state lock")
                .snapshot(storage_source.as_str());
            json_response(
                "200 OK",
                serde_json::json!({
                    "status": "ok",
                    "storage_source": storage_source.as_str(),
                    "supabase": SupabaseMirror::startup_status(),
                    "snapshot": snapshot,
                }),
            )
        }
        ("GET", "/v1/status") => {
            let snapshot = state
                .lock()
                .expect("state lock")
                .snapshot(storage_source.as_str());
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
        ("GET", "/v1/job-events") => {
            let snapshot = state.lock().expect("state lock").job_events_snapshot();
            json_response("200 OK", snapshot)
        }
        ("GET", "/v1/credits") => {
            let snapshot = state.lock().expect("state lock").credits_snapshot();
            json_response("200 OK", snapshot)
        }
        ("GET", "/v1/jobs/next") => {
            if let Some(node_id) = query_param(query, "node_id") {
                let mut guard = state.lock().expect("state lock");
                let claim = guard.claim_job(node_id, now_unix_seconds());
                if let Some(job) = claim.job.as_ref() {
                    let event = guard.record_job_event(
                        Some(node_id.to_string()),
                        Some(job.job_id.clone()),
                        "job_claimed",
                        serde_json::to_value(job).expect("json"),
                        now_unix_seconds(),
                    );
                    if let Some(db) = supabase.as_ref() {
                        if let Err(error) = db.record_job_event(
                            event.node_id.as_deref(),
                            event.job_id.as_deref(),
                            &event.event_type,
                            event.payload.clone(),
                        ) {
                            eprintln!("database claim sync skipped: {error}");
                        }
                    }
                }
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                json_response("200 OK", serde_json::to_value(claim).expect("json"))
            } else {
                text_response("400 Bad Request", "missing node_id")
            }
        }
        ("POST", "/v1/register") => {
            match serde_json::from_str::<AgentRegistration>(&request.body) {
                Ok(registration) => {
                    let registration_clone = registration.clone();
                    let mut guard = state.lock().expect("state lock");
                    let record = guard.register(registration);
                    let event = guard.record_job_event(
                        Some(record.node_id.clone()),
                        None,
                        "registration",
                        serde_json::to_value(&record).expect("json"),
                        now_unix_seconds(),
                    );
                    if let Err(error) = save_state(&guard) {
                        eprintln!("failed to save control-plane state: {error}");
                    }
                    if let Some(db) = supabase.as_ref() {
                        if let Err(error) = db.record_registration(&registration_clone) {
                            eprintln!("database registration sync skipped: {error}");
                        }
                        if let Err(error) = db.record_job_event(
                            event.node_id.as_deref(),
                            event.job_id.as_deref(),
                            &event.event_type,
                            event.payload.clone(),
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
            }
        }
        ("POST", "/v1/heartbeat") => match serde_json::from_str::<Heartbeat>(&request.body) {
            Ok(heartbeat) => {
                let heartbeat_clone = heartbeat.clone();
                let mut guard = state.lock().expect("state lock");
                let record = guard.heartbeat(heartbeat, now_unix_seconds());
                let event = guard.record_job_event(
                    Some(record.node_id.clone()),
                    None,
                    "heartbeat",
                    serde_json::to_value(&record).expect("json"),
                    now_unix_seconds(),
                );
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Err(error) = db.record_heartbeat(&heartbeat_clone) {
                        eprintln!("database heartbeat sync skipped: {error}");
                    }
                    if let Err(error) = db.record_job_event(
                        event.node_id.as_deref(),
                        event.job_id.as_deref(),
                        &event.event_type,
                        event.payload.clone(),
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
                let event = guard.record_job_event(
                    None,
                    Some(record.job_id.clone()),
                    "job_submitted",
                    serde_json::to_value(&record).expect("json"),
                    now_unix_seconds(),
                );
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                if let Some(db) = supabase.as_ref() {
                    if let Err(error) = db.record_job(&record) {
                        eprintln!("database job sync skipped: {error}");
                    }
                    if let Err(error) = db.record_job_event(
                        event.node_id.as_deref(),
                        event.job_id.as_deref(),
                        &event.event_type,
                        event.payload.clone(),
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
        ("POST", "/v1/chat/completions") => {
            match serde_json::from_str::<ChatCompletionRequest>(&request.body) {
                Ok(request_body) => {
                    if request_body.stream.unwrap_or(false) {
                        if let Err(error) = stream.write_all(
                            json_response(
                                "400 Bad Request",
                                serde_json::json!({
                                    "error": "streaming chat completions are not supported yet"
                                }),
                            )
                            .as_bytes(),
                        ) {
                            eprintln!("failed to write response: {error}");
                        }
                        return;
                    }

                    let (system_prompt, prompt) = chat_messages_to_prompt(&request_body.messages);
                    let job_request = JobRequest {
                        request_id: format!("chatcmpl-{}", Uuid::new_v4().simple()),
                        prompt,
                        preferred_backend: crate::contracts::Backend::Auto,
                        model: Some(request_body.model.clone()),
                        system_prompt,
                        max_tokens: request_body.max_tokens,
                        temperature: request_body.temperature,
                        top_p: request_body.top_p,
                        seed: request_body.seed,
                    };

                    let mut guard = state.lock().expect("state lock");
                    let record = guard.submit_job(job_request, now_unix_seconds());
                    let event = guard.record_job_event(
                        None,
                        Some(record.job_id.clone()),
                        "chat_completion_submitted",
                        serde_json::to_value(&record).expect("json"),
                        now_unix_seconds(),
                    );
                    if let Err(error) = save_state(&guard) {
                        eprintln!("failed to save control-plane state: {error}");
                    }
                    if let Some(db) = supabase.as_ref() {
                        if let Err(error) = db.record_job(&record) {
                            eprintln!("database chat completion sync skipped: {error}");
                        }
                        if let Err(error) = db.record_job_event(
                            event.node_id.as_deref(),
                            event.job_id.as_deref(),
                            &event.event_type,
                            event.payload.clone(),
                        ) {
                            eprintln!("database chat completion event skipped: {error}");
                        }
                    }

                    let response = ChatCompletionResponse {
                        id: format!("chatcmpl-{}", Uuid::new_v4().simple()),
                        object: "chat.completion".to_string(),
                        created: now_unix_seconds_u64(),
                        model: request_body.model,
                        choices: vec![ChatCompletionChoice {
                            index: 0,
                            message: ChatCompletionChoiceMessage {
                                role: "assistant".to_string(),
                                content: String::new(),
                            },
                            finish_reason: "queued".to_string(),
                        }],
                        opengpu: ChatCompletionOpenGpu {
                            job_id: record.job_id.clone(),
                            request_id: record.request_id.clone(),
                            status: record.status.to_string(),
                        },
                    };

                    json_response("200 OK", serde_json::to_value(response).expect("json"))
                }
                Err(error) => json_response(
                    "400 Bad Request",
                    serde_json::json!({ "error": error.to_string() }),
                ),
            }
        }
        ("POST", "/v1/jobs/complete") => match serde_json::from_str::<JobCompletion>(&request.body)
        {
            Ok(completion) => {
                let completion_clone = completion.clone();
                let mut guard = state.lock().expect("state lock");
                let record = guard.complete_job(completion, now_unix_seconds());
                if let Some(job) = record.as_ref() {
                    let completed_at = job.completed_at.clone().unwrap_or_else(now_unix_seconds);
                    let event_type = if matches!(job.status, crate::contracts::JobStatus::Completed)
                    {
                        "job_completed"
                    } else {
                        "job_failed"
                    };
                    guard.record_job_event(
                        job.assigned_node_id.clone(),
                        Some(job.job_id.clone()),
                        event_type,
                        serde_json::to_value(job).expect("json"),
                        now_unix_seconds(),
                    );
                    if matches!(job.status, crate::contracts::JobStatus::Completed) {
                        if let Some(award) = guard.award_job_reward(job, completed_at) {
                            let award_event = guard.record_job_event(
                                award.device_id.clone(),
                                award.job_id.clone(),
                                "credit_awarded",
                                serde_json::to_value(&award).expect("json"),
                                award.created_at.clone(),
                            );
                            if let Some(db) = supabase.as_ref() {
                                if let Err(error) = db.record_credit_award(&award) {
                                    eprintln!("database credit sync skipped: {error}");
                                }
                                if let Err(error) = db.record_job_event(
                                    award_event.node_id.as_deref(),
                                    award_event.job_id.as_deref(),
                                    &award_event.event_type,
                                    award_event.payload.clone(),
                                ) {
                                    eprintln!("database credit event skipped: {error}");
                                }
                            }
                        }
                    }
                }
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
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|arg| arg.as_str()) == Some("migrate") {
        match std::env::var("DATABASE_URL") {
            Ok(database_url) => match apply_migrations(&database_url) {
                Ok(applied) => {
                    if applied.is_empty() {
                        println!("no migrations to apply");
                    } else {
                        for migration in applied {
                            println!(
                                "applied {}_{} ({})",
                                migration.version,
                                migration.name,
                                migration.path.display()
                            );
                        }
                    }
                }
                Err(error) => {
                    eprintln!("migration failed: {error}");
                    std::process::exit(1);
                }
            },
            Err(_) => {
                eprintln!("DATABASE_URL is required for migrate");
                std::process::exit(1);
            }
        }
        return;
    }

    let supabase = SupabaseMirror::from_env();
    let listener = TcpListener::bind("127.0.0.1:8787").expect("bind control plane");
    let (restored_state, storage_source) = match supabase.as_ref() {
        Some(db) => match db.restore_state() {
            Ok(state) => {
                println!("restore: supabase");
                (state, StorageSource::Supabase)
            }
            Err(error) => {
                eprintln!("supabase restore skipped: {error}");
                (
                    load_state().ok().flatten().unwrap_or_default(),
                    StorageSource::LocalJsonFallback,
                )
            }
        },
        None => {
            println!("restore: local-json");
            (
                load_state().ok().flatten().unwrap_or_default(),
                StorageSource::LocalJsonOnly,
            )
        }
    };
    let state = Arc::new(Mutex::new(restored_state));

    println!("NovusX control plane listening on http://127.0.0.1:8787");
    println!("supabase: {}", SupabaseMirror::startup_status());
    println!("storage_source: {}", storage_source.as_str());
    if let Ok(database_url) = std::env::var("DATABASE_URL") {
        match applied_migrations(&database_url) {
            Ok(applied) => println!("migrations: {} applied", applied.len()),
            Err(error) => eprintln!("migration status unavailable: {error}"),
        }
    }
    println!(
        "operatorAuth: {}",
        if operator_auth_token().is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
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
                handle_connection(stream, state, supabase.as_ref(), storage_source);
            }
            Err(error) => eprintln!("incoming connection error: {error}"),
        }
    }
}
