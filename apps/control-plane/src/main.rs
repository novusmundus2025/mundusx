mod contracts;
mod state;

use contracts::{AgentRegistration, Heartbeat, JobCompletion, JobRequest};
use state::{load_state, save_state, ControlPlaneState};
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

fn control_plane_home(state: &ControlPlaneState) -> String {
    let snapshot = state.snapshot();
    let nodes = snapshot["online_count"].as_u64().unwrap_or(0);
    let paused = snapshot["paused_count"].as_u64().unwrap_or(0);
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
        <div class="stat"><span>Queued jobs</span><strong>{queued}</strong></div>
        <div class="stat"><span>Assigned jobs</span><strong>{assigned}</strong></div>
        <div class="stat"><span>Completed jobs</span><strong>{completed}</strong></div>
        <div class="stat"><span>Failed jobs</span><strong>{failed}</strong></div>
      </div>
      <p>Useful endpoints: <a href="/health">/health</a>, <a href="/v1/status">/v1/status</a>, <a href="/v1/nodes">/v1/nodes</a>, <a href="/v1/jobs">/v1/jobs</a></p>
    </main>
  </body>
</html>"#
    )
}

fn parse_request(request: &str) -> (String, String, String) {
    let mut lines = request.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
    (method, path, body)
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

fn handle_connection(mut stream: TcpStream, state: Arc<Mutex<ControlPlaneState>>) {
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
    let (method, path, body) = parse_request(&request_text);
    let (clean_path, query) = split_path_and_query(&path);

    let response = match (method.as_str(), clean_path) {
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
                json_response("200 OK", serde_json::to_value(claim).expect("json"))
            } else {
                text_response("400 Bad Request", "missing node_id")
            }
        }
        ("POST", "/v1/register") => match serde_json::from_str::<AgentRegistration>(&body) {
            Ok(registration) => {
                let mut guard = state.lock().expect("state lock");
                let record = guard.register(registration);
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                json_response("200 OK", serde_json::to_value(record).expect("json"))
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        ("POST", "/v1/heartbeat") => match serde_json::from_str::<Heartbeat>(&body) {
            Ok(heartbeat) => {
                let mut guard = state.lock().expect("state lock");
                let record = guard.heartbeat(heartbeat, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                json_response("200 OK", serde_json::to_value(record).expect("json"))
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        ("POST", "/v1/jobs") => match serde_json::from_str::<JobRequest>(&body) {
            Ok(request) => {
                let mut guard = state.lock().expect("state lock");
                let record = guard.submit_job(request, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
                }
                json_response("200 OK", serde_json::to_value(record).expect("json"))
            }
            Err(error) => json_response(
                "400 Bad Request",
                serde_json::json!({ "error": error.to_string() }),
            ),
        },
        ("POST", "/v1/jobs/complete") => match serde_json::from_str::<JobCompletion>(&body) {
            Ok(completion) => {
                let mut guard = state.lock().expect("state lock");
                let record = guard.complete_job(completion, now_unix_seconds());
                if let Err(error) = save_state(&guard) {
                    eprintln!("failed to save control-plane state: {error}");
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
    let listener = TcpListener::bind("127.0.0.1:8787").expect("bind control plane");
    let state = Arc::new(Mutex::new(load_state().ok().flatten().unwrap_or_default()));

    println!("OpenGPU control plane listening on http://127.0.0.1:8787");
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
                handle_connection(stream, state);
            }
            Err(error) => eprintln!("incoming connection error: {error}"),
        }
    }
}
