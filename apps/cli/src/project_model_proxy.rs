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
                        let _ = request.respond(response);
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

fn open_remote_stream(
    remote: &str,
    token: &str,
    task_id: &str,
    connection_id: &str,
    mut body: Value,
) -> Result<ureq::Response, String> {
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
