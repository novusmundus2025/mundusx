use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server, StatusCode};
use uuid::Uuid;

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
                    let _ = request.respond(json_response(StatusCode(200), json!({"object":"list","data":[{"id":"mundusx-agnostic","object":"model","owned_by":"mundusx"}]})));
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
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|_| {
                        serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())
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
                            StatusCode(502),
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
    body["protocol"] = json!("mundusx-project-agent/v1");
    body["project_task_id"] = json!(task_id);
    body["connection_id"] = json!(connection_id);
    body["idempotency_key"] = json!(Uuid::new_v4().to_string());
    body["model"] = json!("mundusx-agnostic");
    let accepted: Value = ureq::post(&format!("{remote}/jobs"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(remote_error)?
        .into_json()
        .map_err(|error| error.to_string())?;
    let job_id = accepted["job_id"]
        .as_str()
        .ok_or("model gateway did not return job_id")?;
    let mut delay = 250;
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = ureq::post(&format!("{remote}/jobs/{job_id}/cancel"))
                .set("Authorization", &format!("Bearer {token}"))
                .call();
            return Err("project model job cancelled".to_string());
        }
        let state: Value = ureq::get(&format!("{remote}/jobs/{job_id}"))
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(remote_error)?
            .into_json()
            .map_err(|error| error.to_string())?;
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
                return Err(state["error"]["message"]
                    .as_str()
                    .unwrap_or(terminal)
                    .to_string())
            }
        }
    }
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
