use serde_json::{json, Value};
use std::io::{self, Read};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use tiny_http::{Header, Method, Response, Server, StatusCode};
use uuid::Uuid;

const MODEL_CONTEXT_TOKENS: u32 = 131_072;
const MAX_MODEL_BODY_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
struct ProxyError {
    status: u16,
    message: String,
}

fn model_catalog() -> Value {
    json!({"object":"list","data":[{"id":"mundusx-agnostic","object":"model","owned_by":"mundusx",
        "context_length": MODEL_CONTEXT_TOKENS, "max_model_len": MODEL_CONTEXT_TOKENS}]})
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
        Self::start_with_progress(remote_base, connector_token, task_id, connection_id, None)
    }

    pub fn start_with_progress(
        remote_base: &str, connector_token: &str, task_id: &str, connection_id: &str,
        progress: Option<std::sync::mpsc::Sender<Value>>,
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
                    let _ = request.respond(json_response(StatusCode(200), model_catalog()));
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
                    .map_err(|error| ProxyError {
                        status: 400,
                        message: error.to_string(),
                    })
                    .and_then(|_| {
                        if bytes.len() as u64 > MAX_MODEL_BODY_BYTES {
                            return Err(ProxyError {
                                status: 413,
                                message: "model request exceeds 32 MiB transport limit".into(),
                            });
                        }
                        serde_json::from_slice::<Value>(&bytes).map_err(|error| ProxyError {
                            status: 400,
                            message: error.to_string(),
                        })
                    })
                    .and_then(|body| {
                        if let Some(sender) = &progress { let _ = sender.send(json!({"type":"model_requested","data":{}})); }
                        open_remote_stream(
                            &remote,
                            &connector_token,
                            &task_id,
                            &connection_id,
                            body,
                        )
                    });
                match result {
                    Ok(upstream) => {
                        let status = StatusCode(upstream.status());
                        let headers = vec![
                            Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                            Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                            Header::from_bytes("X-Accel-Buffering", "no").unwrap(),
                        ];
                        let response = Response::new(
                            status,
                            headers,
                            CancellableReader {
                                inner: upstream.into_reader(),
                                stop: Arc::clone(&worker_stop),
                            },
                            None,
                            None,
                        );
                        let delivered = request.respond(response).is_ok();
                        if let Some(sender) = &progress { let _ = sender.send(json!({"type":if delivered {"model_response_received"} else {"model_failed"},"data":{}})); }
                    }
                    Err(error) => {
                        if let Some(sender) = &progress { let _ = sender.send(json!({"type":"model_failed","data":{}})); }
                        let _ = request.respond(json_response(
                            StatusCode(error.status),
                            json!({"error":{"message":error.message}}),
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

fn open_remote_stream(
    remote: &str,
    token: &str,
    task_id: &str,
    connection_id: &str,
    mut body: Value,
) -> Result<ureq::Response, ProxyError> {
    // Hermes may reuse its client request id for every model turn in one agent
    // run. The control plane treats request ids as idempotency keys, so forwarding
    // that value caused later tool turns to replay the first model decision. Give
    // every HTTP model turn its own gateway id instead.
    let request_id = format!("chatcmpl-{}", Uuid::new_v4().simple());
    body["protocol"] = json!("mundusx-project-agent/v1");
    body["project_task_id"] = json!(task_id);
    body["connection_id"] = json!(connection_id);
    body["request_id"] = json!(request_id);
    body["model"] = json!("mundusx-agnostic");
    body["stream"] = json!(true);
    ureq::post(&format!("{remote}/chat/completions"))
        // Chat sends SSE heartbeats every ten seconds, so this deadline detects
        // a genuinely broken stream without imposing a short model-turn cap.
        .timeout(Duration::from_secs(900))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "text/event-stream")
        .send_json(body)
        .map_err(remote_error)
}

struct CancellableReader {
    inner: Box<dyn Read + Send + Sync>,
    stop: Arc<AtomicBool>,
}

impl Read for CancellableReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Ok(0);
        }
        self.inner.read(buffer)
    }
}

fn remote_error(error: ureq::Error) -> ProxyError {
    match error {
        ureq::Error::Status(code, response) => ProxyError {
            status: code,
            message: format!(
                "model gateway returned {code}: {}",
                response.into_string().unwrap_or_default()
            ),
        },
        ureq::Error::Transport(error) => ProxyError {
            status: 502,
            message: format!("model gateway request failed: {error}"),
        },
    }
}

fn json_response(status: StatusCode, value: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(serde_json::to_vec(&value).unwrap_or_default())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn hermes_discovers_the_real_context_window() {
        let proxy =
            ProjectModelProxy::start("https://invalid.example/v1", "secret", "task", "connection")
                .unwrap();
        let catalog: Value = ureq::get(&format!("{}/models", proxy.base_url()))
            .set("Authorization", &format!("Bearer {}", proxy.credential()))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        assert_eq!(catalog["data"][0]["context_length"], 131_072);
        assert_eq!(catalog["data"][0]["max_model_len"], 131_072);
    }

    #[test]
    fn large_context_passes_through_and_token_errors_are_not_502() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let receiver = thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let mut bytes = Vec::new();
            request.as_reader().read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() > 1024 * 1024);
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                body["messages"][0]["content"].as_str().unwrap().len(),
                2 * 1024 * 1024
            );
            request
                .respond(
                    Response::from_string("maximum context length is 131072 tokens")
                        .with_status_code(400),
                )
                .unwrap();
        });
        let (progress_sender, progress_receiver) = std::sync::mpsc::channel();
        let proxy = ProjectModelProxy::start_with_progress(
            &format!("http://{address}/v1"),
            "secret",
            "task",
            "connection",
            Some(progress_sender),
        )
        .unwrap();
        let result = ureq::post(&format!("{}/chat/completions", proxy.base_url()))
            .set("Authorization", &format!("Bearer {}", proxy.credential()))
            .send_json(json!({"messages":[{"role":"user","content":"x".repeat(2 * 1024 * 1024)}]}));
        match result {
            Err(ureq::Error::Status(400, response)) => {
                assert!(response.into_string().unwrap().contains("131072"))
            }
            _ => panic!("context error must remain HTTP 400"),
        }
        receiver.join().unwrap();
        assert_eq!(progress_receiver.recv_timeout(Duration::from_secs(1)).unwrap()["type"], "model_requested");
        assert_eq!(progress_receiver.recv_timeout(Duration::from_secs(1)).unwrap()["type"], "model_failed");
    }

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
    fn model_turn_uses_the_openai_compatible_streaming_route() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap().to_string();
        let remote = format!("http://{address}/api/agent/model/v1");
        let receiver = thread::spawn(move || {
            let mut request = server.recv().unwrap();
            assert_eq!(request.url(), "/api/agent/model/v1/chat/completions");
            assert!(request.headers().iter().any(|header| {
                header.field.equiv("Authorization") && header.value.as_str() == "Bearer secret"
            }));
            let mut bytes = Vec::new();
            request.as_reader().read_to_end(&mut bytes).unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["stream"], true);
            assert_eq!(body["model"], "mundusx-agnostic");
            assert_eq!(body["project_task_id"], "task");
            assert_eq!(body["connection_id"], "connection");
            assert!(body["request_id"]
                .as_str()
                .unwrap()
                .starts_with("chatcmpl-"));
            request
                .respond(
                    Response::from_string("data: {\"choices\":[]}\n\ndata: [DONE]\n\n")
                        .with_header(
                            Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                        ),
                )
                .unwrap();
        });
        let response = open_remote_stream(
            &remote,
            "secret",
            "task",
            "connection",
            json!({"messages":[{"role":"user","content":"hello"}],"tools":[]}),
        )
        .unwrap();
        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        receiver.join().unwrap();
        assert!(body.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn model_turns_receive_distinct_gateway_request_ids() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap().to_string();
        let remote = format!("http://{address}/api/agent/model/v1");
        let receiver = thread::spawn(move || {
            let mut ids = Vec::new();
            for _ in 0..2 {
                let mut request = server.recv().unwrap();
                let mut bytes = Vec::new();
                request.as_reader().read_to_end(&mut bytes).unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                ids.push(body["request_id"].as_str().unwrap().to_string());
                request
                    .respond(Response::from_string("data: [DONE]\n\n"))
                    .unwrap();
            }
            ids
        });
        for _ in 0..2 {
            open_remote_stream(
                &remote,
                "secret",
                "task",
                "connection",
                json!({"request_id":"reused-by-hermes","messages":[],"tools":[]}),
            )
            .unwrap();
        }
        let ids = receiver.join().unwrap();
        assert_ne!(ids[0], ids[1]);
        assert!(ids.iter().all(|id| id != "reused-by-hermes"));
    }
}
