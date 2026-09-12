use clap::Parser;
use mundusx_agent_core::{
    conversation_transcript, AgentEvent, AgentEventKind, AgentEventStore, AgentRunner,
    AgentSession, ModelError, ModelProvider, ModelRequest, SessionId, SqliteEventStore,
    StaticApprovalProvider, ToolContext,
};
use mundusx_agent_skills::{load_skills, select_skills};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server};

const MODEL_ID: &str = "mundusx-agent";

#[derive(Parser)]
#[command(name = "mundusx-agent-server", version, about = "Local MundusX agent API")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:11436")]
    bind: String,
    #[arg(long, default_value = "http://127.0.0.1:11435")]
    runtime_url: String,
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
struct ChatMessage {
    role: String,
    #[serde(default)]
    content: Value,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    #[serde(default = "default_model")]
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct RuntimeResponse {
    #[serde(default)]
    output: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

struct LocalRuntimeProvider {
    runtime_url: String,
    token: String,
    model: String,
}

impl ModelProvider for LocalRuntimeProvider {
    fn provider_name(&self) -> &str {
        "mundusx-local-runtime"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn complete(&mut self, request: &ModelRequest) -> Result<String, ModelError> {
        let response = ureq::post(&format!(
            "{}/local/v1/chat/completions",
            self.runtime_url.trim_end_matches('/')
        ))
        .set("Authorization", &format!("Bearer {}", self.token))
        .send_json(json!({
            "request_id": format!("agent-turn-{}", uuid::Uuid::new_v4().simple()),
            "prompt": format!("SYSTEM:\n{}\n\nTRANSCRIPT:\n{}", request.system_prompt, request.transcript),
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "force_local": true
        }))
        .map_err(|error| ModelError::new(error.to_string()))?;
        let runtime: RuntimeResponse = response
            .into_json()
            .map_err(|error| ModelError::new(format!("invalid runtime response: {error}")))?;
        if let Some(error) = runtime.error.filter(|value| !value.trim().is_empty()) {
            return Err(ModelError::new(error));
        }
        if let Some(model) = runtime.model {
            self.model = model;
        }
        Ok(runtime.output)
    }
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: ApiError,
}

#[derive(Debug, Serialize)]
struct ApiError {
    message: String,
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'static str,
}

fn default_model() -> String {
    MODEL_ID.to_string()
}

fn config_dir() -> PathBuf {
    std::env::var_os("MUNDUSX_HOME")
        .or_else(|| std::env::var_os("OPENGPU_HOME"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".mundusx")))
        .unwrap_or_else(|| PathBuf::from(".mundusx"))
}

fn json_response(status: u16, value: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let payload = serde_json::to_vec(&value).expect("serialize API response");
    Response::from_data(payload)
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").expect("header"))
}

fn sse_response(value: &Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let content = value["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default();
    let chunk = json!({
        "id": value["id"],
        "object": "chat.completion.chunk",
        "created": value["created"],
        "model": value["model"],
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": content},
            "finish_reason": Value::Null
        }]
    });
    let terminal = json!({
        "id": value["id"],
        "object": "chat.completion.chunk",
        "created": value["created"],
        "model": value["model"],
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    });
    let body = format!("data: {chunk}\n\ndata: {terminal}\n\ndata: [DONE]\n\n");
    Response::from_string(body).with_header(
        Header::from_bytes("Content-Type", "text/event-stream; charset=utf-8").expect("header"),
    )
}

fn api_error(
    status: u16,
    code: &'static str,
    message: impl Into<String>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(
        status,
        serde_json::to_value(ApiErrorBody {
            error: ApiError {
                message: message.into(),
                kind: "mundusx_agent_error",
                code,
            },
        })
        .expect("serialize error"),
    )
}

fn bearer_token(request: &Request) -> Option<&str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Authorization"))
        .and_then(|header| header.value.as_str().strip_prefix("Bearer "))
}

fn configured_api_key() -> Option<String> {
    std::env::var("MUNDUSX_AGENT_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
fn prompt_from_messages(messages: &[ChatMessage]) -> Result<String, String> {
    let lines = messages
        .iter()
        .filter_map(|message| {
            let content = message_text(&message.content);
            (!content.trim().is_empty()).then(|| format!("{}: {content}", message.role))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        Err("messages must contain text content".to_string())
    } else {
        Ok(lines.join("\n\n"))
    }
}

fn current_user_input(messages: &[ChatMessage]) -> Result<String, String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message_text(&message.content))
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "messages must contain a user message".to_string())
}

fn runtime_token() -> Result<String, String> {
    std::env::var("MUNDUSX_RUNTIME_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(Ok)
        .unwrap_or_else(|| {
            let preferred = config_dir().join("local-agent-token");
            let legacy = dirs::home_dir().map(|path| path.join(".opengpu/local-agent-token"));
            [Some(preferred), legacy]
                .into_iter()
                .flatten()
                .find_map(|path| fs::read_to_string(path).ok())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    "local model runtime token is unavailable; start OpenGPU first".to_string()
                })
        })
}

fn next_sequence(store: &SqliteEventStore, session_id: &SessionId) -> Result<u64, String> {
    store
        .events(session_id)
        .map(|events| events.last().map_or(1, |event| event.sequence + 1))
        .map_err(|error| error.to_string())
}

fn handle_chat(
    mut request: Request,
    runtime_url: &str,
    database_path: &std::path::Path,
    context: &ToolContext,
    cancellations: &Arc<Mutex<HashSet<SessionId>>>,
) {
    let mut body = String::new();
    if request
        .as_reader()
        .take(1024 * 1024)
        .read_to_string(&mut body)
        .is_err()
    {
        let _ = request.respond(api_error(
            400,
            "INVALID_REQUEST",
            "could not read request body",
        ));
        return;
    }
    let payload: ChatRequest = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(error) => {
            let _ = request.respond(api_error(400, "INVALID_REQUEST", error.to_string()));
            return;
        }
    };
    let mut store = match SqliteEventStore::open(database_path) {
        Ok(value) => value,
        Err(error) => {
            let _ = request.respond(api_error(500, "SESSION_STORE_FAILED", error.to_string()));
            return;
        }
    };
    let prompt = match current_user_input(&payload.messages) {
        Ok(value) => value,
        Err(error) => {
            let _ = request.respond(api_error(400, "INVALID_MESSAGES", error));
            return;
        }
    };
    let session_id = header_value(&request, "X-MundusX-Session-Id")
        .and_then(|value| SessionId::parse(value).ok())
        .unwrap_or_default();
    cancellations
        .lock()
        .expect("cancellation lock")
        .remove(&session_id);
    let working_directory = context.workspace().display().to_string();
    let session = AgentSession::with_id(session_id.clone(), working_directory, None);
    let (mut sequence, prior_transcript) = {
        if let Err(error) = store.ensure_session(&session) {
            let _ = request.respond(api_error(500, "SESSION_STORE_FAILED", error.to_string()));
            return;
        }
        let existing_events = match store.events(&session_id) {
            Ok(value) => value,
            Err(error) => {
                let _ = request.respond(api_error(500, "SESSION_STORE_FAILED", error.to_string()));
                return;
            }
        };
        let prior_transcript = conversation_transcript(&existing_events);
        let sequence = match next_sequence(&store, &session_id) {
            Ok(value) => value,
            Err(error) => {
                let _ = request.respond(api_error(500, "SESSION_STORE_FAILED", error));
                return;
            }
        };
        if let Err(error) = store.append(&AgentEvent::new(
            session_id.clone(),
            sequence,
            AgentEventKind::UserMessageReceived {
                content: prompt.clone(),
            },
        )) {
            let _ = request.respond(api_error(500, "SESSION_STORE_FAILED", error.to_string()));
            return;
        }
        (sequence, prior_transcript)
    };
    let selected_skills = load_skills(&config_dir().join("skills"))
        .map(|skills| {
            select_skills(&skills, &prompt, 3)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !selected_skills.is_empty() {
        for skill in &selected_skills {
            sequence += 1;
            if let Err(error) = store.append(&AgentEvent::new(
                session_id.clone(),
                sequence,
                AgentEventKind::SkillLoaded {
                    name: skill.name.clone(),
                },
            )) {
                let _ = request.respond(api_error(500, "SESSION_STORE_FAILED", error.to_string()));
                return;
            }
        }
    }
    let token = match runtime_token() {
        Ok(value) => value,
        Err(error) => {
            let _ = request.respond(api_error(503, "LOCAL_RUNTIME_UNAVAILABLE", error));
            return;
        }
    };
    let mut provider = LocalRuntimeProvider {
        runtime_url: runtime_url.to_string(),
        token,
        model: "local-active".to_string(),
    };
    let mut tools =
        match mundusx_agent_tools::full_registry(Default::default(), config_dir().join("agent.db"))
        {
            Ok(value) => value,
            Err(error) => {
                let _ = request.respond(api_error(500, "TOOL_REGISTRY_FAILED", error.to_string()));
                return;
            }
        };
    if let Ok(base_url) = std::env::var("MUNDUSX_CONTROL_PLANE_URL") {
        let delegation = mundusx_agent_tools::ControlPlaneDelegation {
            base_url,
            bearer_token: std::env::var("MUNDUSX_CONTROL_PLANE_TOKEN").ok(),
        };
        if let Err(error) =
            mundusx_agent_tools::register_control_plane_delegation(&mut tools, delegation)
        {
            let _ = request.respond(api_error(500, "TOOL_REGISTRY_FAILED", error.to_string()));
            return;
        }
    }
    let allow_mutations = header_value(&request, "X-MundusX-Allow-Mutations")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let mut approval = if allow_mutations {
        StaticApprovalProvider::allow_mutations()
    } else {
        StaticApprovalProvider::read_only()
    };
    let outcome = {
        let cancelled = || {
            cancellations
                .lock()
                .expect("cancellation lock")
                .contains(&session_id)
        };
        AgentRunner::new(&mut provider, &tools)
            .with_generation(payload.max_tokens, payload.temperature)
            .with_approval_provider(&mut approval)
            .with_instructions(
                selected_skills
                    .iter()
                    .map(|skill| format!("Skill: {}\n{}", skill.name, skill.instructions))
                    .collect(),
            )
            .with_cancellation(&cancelled)
            .run(
                &mut store,
                session_id.clone(),
                sequence + 1,
                &prompt,
                &prior_transcript,
                context,
            )
    };
    let outcome = match outcome {
        Ok(value) => value,
        Err(error) => {
            let _ = request.respond(api_error(502, "AGENT_RUN_FAILED", error.to_string()));
            return;
        }
    };
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let response_value = json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
        "object": "chat.completion",
        "created": created,
        "model": payload.model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": outcome.content},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
        "mundusx": {
            "session_id": session_id.to_string(),
            "runtime_model": provider.model_name(),
            "turns": outcome.turns,
            "tool_calls": outcome.tool_calls
        }
    });
    if payload.stream {
        let _ = request.respond(sse_response(&response_value));
        return;
    }
    let mut response = json_response(200, response_value);
    response.add_header(
        Header::from_bytes("X-MundusX-Session-Id", session_id.to_string()).expect("header"),
    );
    let _ = request.respond(response);
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    if !(args.bind.starts_with("127.0.0.1:") || args.bind.starts_with("[::1]:")) {
        return Err("the agent server must bind to a loopback address".to_string());
    }
    let data_dir = config_dir();
    fs::create_dir_all(&data_dir).map_err(|error| error.to_string())?;
    let database_path = data_dir.join("agent.db");
    SqliteEventStore::open(&database_path).map_err(|error| error.to_string())?;
    let context = ToolContext::new(&args.workspace).map_err(|error| error.to_string())?;
    let cancellations = Arc::new(Mutex::new(HashSet::new()));
    let server = Server::http(&args.bind).map_err(|error| error.to_string())?;
    let api_key = configured_api_key();
    println!("MundusX agent API listening on http://{}", args.bind);
    for request in server.incoming_requests() {
        let api_key = api_key.clone();
        let database_path = database_path.clone();
        let context = context.clone();
        let runtime_url = args.runtime_url.clone();
        let cancellations = Arc::clone(&cancellations);
        std::thread::spawn(move || {
            if api_key
                .as_deref()
                .is_some_and(|key| bearer_token(&request) != Some(key))
            {
                let _ = request.respond(api_error(401, "UNAUTHORIZED", "invalid API key"));
                return;
            }
            match (
                request.method(),
                request.url().split('?').next().unwrap_or(""),
            ) {
                (&Method::Get, "/health") => {
                    let _ = request.respond(json_response(200, json!({"status": "ok"})));
                }
                (&Method::Get, "/v1/models") => {
                    let _ = request.respond(json_response(
                        200,
                        json!({
                            "object": "list",
                            "data": [{"id": MODEL_ID, "object": "model", "owned_by": "mundusx"}]
                        }),
                    ));
                }
                (&Method::Get, path)
                    if path.starts_with("/v1/sessions/") && path.ends_with("/events") =>
                {
                    let id = path
                        .trim_start_matches("/v1/sessions/")
                        .trim_end_matches("/events")
                        .trim_end_matches('/');
                    match SessionId::parse(id) {
                        Ok(session_id) => {
                            let result = SqliteEventStore::open(&database_path)
                                .and_then(|store| store.events(&session_id));
                            match result {
                                Ok(events) => {
                                    let _ = request
                                        .respond(json_response(200, json!({"data": events})));
                                }
                                Err(error) => {
                                    let _ = request.respond(api_error(
                                        500,
                                        "SESSION_STORE_FAILED",
                                        error.to_string(),
                                    ));
                                }
                            }
                        }
                        Err(_) => {
                            let _ = request.respond(api_error(
                                400,
                                "INVALID_SESSION_ID",
                                "invalid session id",
                            ));
                        }
                    }
                }
                (&Method::Post, path)
                    if path.starts_with("/v1/sessions/") && path.ends_with("/cancel") =>
                {
                    let id = path
                        .trim_start_matches("/v1/sessions/")
                        .trim_end_matches("/cancel")
                        .trim_end_matches('/');
                    match SessionId::parse(id) {
                        Ok(session_id) => {
                            cancellations
                                .lock()
                                .expect("cancellation lock")
                                .insert(session_id);
                            let _ = request.respond(json_response(202, json!({"cancelled": true})));
                        }
                        Err(_) => {
                            let _ = request.respond(api_error(
                                400,
                                "INVALID_SESSION_ID",
                                "invalid session id",
                            ));
                        }
                    }
                }
                (&Method::Post, "/v1/chat/completions") => {
                    handle_chat(
                        request,
                        &runtime_url,
                        &database_path,
                        &context,
                        &cancellations,
                    );
                }
                _ => {
                    let _ = request.respond(api_error(404, "NOT_FOUND", "route not found"));
                }
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{message_text, prompt_from_messages, ChatMessage};
    use serde_json::json;

    #[test]
    fn accepts_text_and_openai_multimodal_text_parts() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: json!("Work locally"),
            },
            ChatMessage {
                role: "user".to_string(),
                content: json!([
                    {"type": "text", "text": "Inspect"},
                    {"type": "text", "text": "this repository"}
                ]),
            },
        ];
        assert_eq!(
            prompt_from_messages(&messages).expect("prompt"),
            "system: Work locally\n\nuser: Inspect\nthis repository"
        );
        assert_eq!(message_text(&json!({"unsupported": true})), "");
    }
}
