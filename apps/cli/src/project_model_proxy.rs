use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use uuid::Uuid;
use std::io::Read;

const MAX_MODEL_BODY_BYTES: u64 = 32 * 1024 * 1024;

fn proxy_error_status(error: &str) -> u16 {
    if error.starts_with("model gateway returned 413:") || error == "model request exceeds 32 MiB transport limit" { return 413; }
    if error.starts_with("model gateway returned 422:") { return 422; }
    if error.starts_with("model gateway returned 400:") || error.starts_with("invalid json body:")
        || error.to_ascii_lowercase().contains("maximum context length") || error.contains("context_length_exceeded") { return 400; }
    502
}

pub struct ProjectModelProxy {
    address: String,
    credential: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ProjectModelProxy {
    pub fn start(
        remote_base: &str,
        connector_token: &str,
        task_id: &str,
        connection_id: &str,
    ) -> Result<Self, String> {
        let server = Server::http("127.0.0.1:0")
            .map_err(|error| format!("could not start project model proxy: {error}"))?;
        let address = server
            .server_addr()
            .to_ip()
            .ok_or("project model proxy did not bind TCP")?
            .to_string();
        let credential = format!("mx_local_{}", Uuid::new_v4().simple());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let remote = remote_base.trim_end_matches('/').to_string();
        let connector_token = connector_token.to_string();
        let task_id = task_id.to_string();
        let connection_id = connection_id.to_string();
        let expected = credential.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(250)) else {
                    continue;
                };
                let authorized = request.headers().iter().any(|header| {
                    header.field.equiv("Authorization")
                        && header.value.as_str() == format!("Bearer {expected}")
                });
                if !authorized {
                    let _ = request.respond(json_response(
                        StatusCode(401),
                        json!({"error":{"message":"invalid local proxy credential"}}),
                    ));
                    continue;
                }
                if request.method() == &Method::Get && request.url() == "/v1/models" {
                    let _ = request.respond(json_response(StatusCode(200), json!({"object":"list","data":[{"id":"mundusx-agnostic","object":"model","owned_by":"mundusx","context_length":131072,"max_model_len":131072}]})));
                    continue;
                }
                if request.method() != &Method::Post || request.url() != "/v1/chat/completions" {
                    let _ = request.respond(json_response(
                        StatusCode(404),
                        json!({"error":{"message":"unknown local model route"}}),
                    ));
                    continue;
                }
                let mut bytes = Vec::new();
                let result = request
                    .as_reader()
                    .take(MAX_MODEL_BODY_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|_| {
                        if bytes.len() as u64 > MAX_MODEL_BODY_BYTES { return Err("model request exceeds 32 MiB transport limit".to_string()); }
                        serde_json::from_slice::<Value>(&bytes).map_err(|error| format!("invalid json body: {error}"))
                    })
                    .and_then(|body| {
                        execute_remote_job(
                            &remote,
                            &connector_token,
                            &task_id,
                            &connection_id,
                            body,
                            &worker_stop,
                        )
                    });
                match result {
                    Ok(completion) => {
                        let _ = request.respond(sse_response(completion));
                    }
                    Err(error) => {
                        let _ = request.respond(json_response(
                            StatusCode(proxy_error_status(&error)),
                            json!({"error":{"message":error}}),
                        ));
                    }
                }
            }
        });
        Ok(Self {
            address,
            credential,
            stop,
            thread: Some(worker),
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }
    pub fn credential(&self) -> &str {
        &self.credential
    }
}

impl Drop for ProjectModelProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

fn execute_remote_job(
    remote: &str,
    token: &str,
    task_id: &str,
    connection_id: &str,
    mut body: Value,
    stop: &AtomicBool,
) -> Result<Value, String> {
    const MODEL_TURN_BUDGET: Duration = Duration::from_secs(180);
    const RECOVERY_ATTEMPTS: usize = 3;
    let idempotency_key = Uuid::new_v4().to_string();
    let deadline = Instant::now() + MODEL_TURN_BUDGET;
    body["protocol"] = json!("mundusx-project-agent/v1");
    body["project_task_id"] = json!(task_id);
    body["connection_id"] = json!(connection_id);
    body["idempotency_key"] = json!(idempotency_key);
    body["model"] = json!("mundusx-agnostic");
    for recovery_attempt in 0..RECOVERY_ATTEMPTS {
        let accepted = retry_remote_json(stop, || {
            ureq::post(&format!("{remote}/jobs"))
                .timeout(Duration::from_secs(30))
                .set("Authorization", &format!("Bearer {token}"))
                .send_json(body.clone())
        })?;
        let job_id = accepted["job_id"]
            .as_str()
            .ok_or("model gateway did not return job_id")?
            .to_string();
        let mut delay = 250;
        let mut poll_errors = 0;
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = ureq::post(&format!("{remote}/jobs/{job_id}/cancel"))
                    .timeout(Duration::from_secs(10))
                    .set("Authorization", &format!("Bearer {token}"))
                    .call();
                return Err("project model job cancelled".to_string());
            }
            if Instant::now() >= deadline {
                let _ = ureq::post(&format!("{remote}/jobs/{job_id}/cancel"))
                    .timeout(Duration::from_secs(10))
                    .set("Authorization", &format!("Bearer {token}"))
                    .call();
                return Err("MundusX model turn did not respond within 3 minutes".to_string());
            }
            let state = match ureq::get(&format!("{remote}/jobs/{job_id}"))
                .timeout(Duration::from_secs(30))
                .set("Authorization", &format!("Bearer {token}"))
                .call()
            {
                Ok(response) => {
                    poll_errors = 0;
                    response
                        .into_json::<Value>()
                        .map_err(|error| error.to_string())?
                }
                Err(_error) if poll_errors < 3 => {
                    poll_errors += 1;
                    thread::sleep(Duration::from_secs(poll_errors));
                    continue;
                }
                Err(error) => return Err(remote_error(error)),
            };
            match state["status"].as_str().unwrap_or("failed") {
                "completed" => return Ok(state["result"].clone()),
                "queued" | "running" => {
                    delay = state["retry_after_ms"]
                        .as_u64()
                        .unwrap_or(delay)
                        .clamp(100, 2000);
                    thread::sleep(Duration::from_millis(delay));
                    delay = (delay * 2).min(2000);
                }
                terminal => {
                    let retryable = state["error"]["retryable"].as_bool().unwrap_or(false);
                    if retryable && recovery_attempt + 1 < RECOVERY_ATTEMPTS {
                        thread::sleep(Duration::from_secs(2 * (recovery_attempt + 1) as u64));
                        break;
                    }
                    return Err(state["error"]["message"]
                        .as_str()
                        .unwrap_or(terminal)
                        .to_string());
                }
            }
        }
    }
    Err("project model job exhausted recovery attempts".to_string())
}

fn retry_remote_json<F>(stop: &AtomicBool, mut request: F) -> Result<Value, String>
where
    F: FnMut() -> Result<ureq::Response, ureq::Error>,
{
    let mut last_error = None;
    for attempt in 0..3 {
        if stop.load(Ordering::Relaxed) {
            return Err("project model job cancelled".to_string());
        }
        match request() {
            Ok(response) => {
                return response
                    .into_json()
                    .map_err(|error| format!("model gateway returned invalid JSON: {error}"));
            }
            Err(error) => last_error = Some(remote_error(error)),
        }
        if attempt < 2 {
            thread::sleep(Duration::from_secs(attempt + 1));
        }
    }
    Err(last_error.unwrap_or_else(|| "model gateway request failed".to_string()))
}

fn remote_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => format!(
            "model gateway returned {code}: {}",
            response.into_string().unwrap_or_default()
        ),
        ureq::Error::Transport(error) => format!("model gateway request failed: {error}"),
    }
}

fn json_response(status: StatusCode, value: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(serde_json::to_vec(&value).unwrap_or_default())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn sse_response(completion: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let choice = &completion["choices"][0];
    let message = &choice["message"];
    let mut delta = json!({"role":"assistant"});
    if !message["tool_calls"].is_null() {
        delta["tool_calls"] = message["tool_calls"].clone();
    } else {
        delta["content"] = message["content"].clone();
    }
    let chunk = json!({
        "id": completion["id"], "object":"chat.completion.chunk",
        "created": completion["created"], "model":"mundusx-agnostic",
        "choices":[{"index":0,"delta":delta,"finish_reason":choice["finish_reason"]}]
    });
    let body = format!("data: {}\n\ndata: [DONE]\n\n", chunk);
    Response::from_data(body.into_bytes())
        .with_status_code(StatusCode(200))
        .with_header(Header::from_bytes("Content-Type", "text/event-stream").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_errors_are_not_retryable_outages() {
        assert_eq!(proxy_error_status("model gateway returned 413: too large"), 413);
        assert_eq!(proxy_error_status("maximum context length is 131072"), 400);
        assert_eq!(proxy_error_status("model gateway returned 422: invalid request"), 422);
        assert_eq!(proxy_error_status("connection timed out"), 502);
    }
    use std::io::Read;

    #[test]
    fn proxy_binds_only_to_loopback_and_uses_ephemeral_credentials() {
        // Remote access is lazy; starting the proxy never transmits credentials.
        let proxy = ProjectModelProxy::start(
            "https://invalid.example/v1",
            "secret",
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
        assert!(proxy.address.starts_with("127.0.0.1:"));
        assert!(proxy.credential().starts_with("mx_local_"));
    }

    #[test]
    fn tool_completion_is_encoded_as_an_sse_tool_delta() {
        let response = sse_response(
            json!({"id":"x","created":1,"choices":[{"message":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}),
        );
        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert!(body.contains("\"tool_calls\""));
        assert!(body.ends_with("data: [DONE]\n\n"));
    }
}
