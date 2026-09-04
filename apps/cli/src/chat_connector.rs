use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

pub struct ConnectorOptions {
    pub chat_url: String,
    pub token: String,
    pub device_name: String,
    pub workspace: PathBuf,
}

pub fn validate_chat_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let secure = value.starts_with("https://");
    let loopback = value.starts_with("http://127.0.0.1") || value.starts_with("http://localhost");
    if !secure && !loopback {
        return Err("chat URL must use HTTPS (HTTP is allowed only for localhost)".to_string());
    }
    Ok(value.to_string())
}

fn connection_file(data_dir: &Path) -> PathBuf {
    data_dir.join("chat-connection.json")
}

fn connection_id(data_dir: &Path) -> Result<String, String> {
    let path = connection_file(data_dir);
    if let Ok(value) = fs::read_to_string(&path) {
        if let Some(id) = serde_json::from_str::<Value>(&value)
            .ok()
            .and_then(|value| value["connection_id"].as_str().map(str::to_string))
            .filter(|id| Uuid::parse_str(id).is_ok())
        {
            return Ok(id);
        }
    }
    let id = Uuid::new_v4().to_string();
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("could not create MundusX data directory: {error}"))?;
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({"connection_id": id})).unwrap(),
    )
    .map_err(|error| format!("could not save connection identity: {error}"))?;
    Ok(id)
}

fn post_remote(url: &str, token: &str, path: &str, body: Value) -> Result<Value, String> {
    ureq::post(&format!("{url}{path}"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(|error| format!("Chat connector request failed: {error}"))?
        .into_json()
        .map_err(|error| format!("Chat connector returned invalid JSON: {error}"))
}

fn local_request(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let base = super::agent_url();
    let mut request = match method {
        "POST" => ureq::post(&format!("{}{path}", base.trim_end_matches('/'))),
        _ => ureq::get(&format!("{}{path}", base.trim_end_matches('/'))),
    };
    if let Some(key) = super::api_key() {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }
    let response = match body {
        Some(value) => request.send_json(value),
        None => request.call(),
    }
    .map_err(|error| format!("local agent request failed: {error}"))?;
    response
        .into_json()
        .map_err(|error| format!("local agent returned invalid JSON: {error}"))
}

fn sanitized_events(session_id: &str) -> Vec<Value> {
    let Ok(payload) = local_request("GET", &format!("/v1/sessions/{session_id}/events"), None)
    else {
        return Vec::new();
    };
    payload["data"].as_array().into_iter().flatten().filter_map(|item| {
        let sequence = item["sequence"].as_u64()?;
        let kind = item["kind"].as_object()?;
        let event_type = normalized_event_type(kind.get("type")?.as_str()?)?;
        let data = kind.get("data").cloned().unwrap_or_else(|| json!({}));
        let metadata = match event_type {
            "model_requested" => json!({"provider": data["provider"], "model": data["model"]}),
            "tool_proposed" => json!({"tool": data["tool"]}),
            "approval_resolved" => json!({"approved": data["approved"]}),
            "tool_completed" => json!({"is_error": data["is_error"]}),
            "context_compacted" => json!({"original_chars": data["original_chars"], "retained_chars": data["retained_chars"]}),
            "skill_loaded" => json!({"name": data["name"]}),
            "task_delegated" => json!({"destination": data["destination"]}),
            _ => json!({}),
        };
        Some(json!({"sequence": sequence, "event": {"type": event_type, "metadata": metadata}}))
    }).collect()
}

fn normalized_event_type(value: &str) -> Option<&'static str> {
    Some(match value {
        "model_requested" => "model.requested",
        "tool_proposed" => "tool.proposed",
        "approval_resolved" => "approval.resolved",
        "tool_completed" => "tool.completed",
        "context_compacted" => "context.compacted",
        "task_delegated" => "task.delegated",
        "session_created" | "user_message" | "assistant_message" | "skill_loaded" => {
            "runtime.progress"
        }
        "cancelled" => "runtime.completed",
        _ => return None,
    })
}

fn deepagents_executable() -> Option<PathBuf> {
    std::env::var_os("MUNDUSX_DEEPAGENTS_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn run_deepagents(options: &ConnectorOptions, task: &Value) -> Result<Value, String> {
    let executable = deepagents_executable()
        .ok_or("Deep Agents runtime was selected but MUNDUSX_DEEPAGENTS_BIN is unavailable")?;
    let mut child = Command::new(&executable)
        .arg("run-json")
        .arg("--workspace")
        .arg(&options.workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start Deep Agents runtime: {error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("Deep Agents runtime stdin is unavailable")?
        .write_all(task.to_string().as_bytes())
        .map_err(|error| format!("could not send task to Deep Agents runtime: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Deep Agents runtime failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Deep Agents runtime exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Deep Agents runtime returned invalid JSON: {error}"))
}

fn run_task(options: &ConnectorOptions, connection_id: &str, task: &Value) -> Result<(), String> {
    let task_id = task["task_id"]
        .as_str()
        .ok_or("task_id is missing")?
        .to_string();
    let session_id = task["session_id"]
        .as_str()
        .ok_or("session_id is missing")?
        .to_string();
    let prompt = task["prompt"]
        .as_str()
        .ok_or("task prompt is missing")?
        .to_string();
    let allow_mutations = task["allow_mutations"].as_bool().unwrap_or(false);
    let stop = Arc::new(AtomicBool::new(false));
    let heartbeat_stop = Arc::clone(&stop);
    let heartbeat_url = options.chat_url.clone();
    let heartbeat_token = options.token.clone();
    let heartbeat_connection = connection_id.to_string();
    let heartbeat_task = task_id.clone();
    let heartbeat_session = session_id.clone();
    let heartbeat = thread::spawn(move || {
        while !heartbeat_stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(10));
            if heartbeat_stop.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(state) = post_remote(
                &heartbeat_url,
                &heartbeat_token,
                &format!("/api/agent/connector/tasks/{heartbeat_task}/heartbeat"),
                json!({"connection_id": heartbeat_connection}),
            ) {
                if state["state"] == "cancelled" {
                    let _ = local_request(
                        "POST",
                        &format!("/v1/sessions/{heartbeat_session}/cancel"),
                        None,
                    );
                    break;
                }
            }
        }
    });

    let response = if task["runtime_selected"] == "deepagents" {
        run_deepagents(options, task)
    } else {
        match super::post_chat(&prompt, Some(&session_id), allow_mutations) {
            Ok(value) => Ok(value),
            Err(first_error) => {
                super::spawn_server(&options.workspace)
                    .map_err(|start_error| format!("{first_error}; {start_error}"))?;
                super::post_chat(&prompt, Some(&session_id), allow_mutations)
            }
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = heartbeat.join();

    let events = response
        .as_ref()
        .ok()
        .and_then(|value| value["events"].as_array().cloned())
        .unwrap_or_else(|| sanitized_events(&session_id));
    if !events.is_empty() {
        let _ = post_remote(
            &options.chat_url,
            &options.token,
            &format!("/api/agent/connector/tasks/{task_id}/events"),
            json!({"events": events}),
        );
    }
    match response {
        Ok(value) => {
            let content = value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .or_else(|| value["output"].as_str())
                .ok_or("local agent response did not contain assistant content")?;
            post_remote(
                &options.chat_url,
                &options.token,
                &format!("/api/agent/connector/tasks/{task_id}/complete"),
                json!({"status": "completed", "result": {"content": content, "session_id": session_id}}),
            )?;
        }
        Err(error) => {
            post_remote(
                &options.chat_url,
                &options.token,
                &format!("/api/agent/connector/tasks/{task_id}/complete"),
                json!({"status": "failed", "error": error}),
            )?;
        }
    }
    Ok(())
}

pub fn connect(options: ConnectorOptions, data_dir: &Path) -> Result<(), String> {
    let connection_id = connection_id(data_dir)?;
    post_remote(
        &options.chat_url,
        &options.token,
        "/api/agent/connector/register",
        json!({
            "connection_id": connection_id,
            "device_name": options.device_name,
            "capabilities": {
                "protocol": "mundusx-agent-bridge/v1",
                "mutations": false,
                "agent_runtimes": if deepagents_executable().is_some() { json!(["native", "deepagents"]) } else { json!(["native"]) }
            }
        }),
    )?;
    eprintln!(
        "Connected local MundusX agent to {}. Press Ctrl+C to stop.",
        options.chat_url
    );
    loop {
        match post_remote(
            &options.chat_url,
            &options.token,
            "/api/agent/connector/tasks/next",
            json!({"connection_id": connection_id}),
        ) {
            Ok(payload) if !payload["task"].is_null() => {
                if let Err(error) = run_task(&options, &connection_id, &payload["task"]) {
                    eprintln!("local task failed: {error}");
                }
            }
            Ok(_) => thread::sleep(Duration::from_secs(2)),
            Err(error) => {
                eprintln!("connection interrupted: {error}; retrying");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalized_event_type, validate_chat_url};

    #[test]
    fn connector_requires_secure_remote_transport() {
        assert!(validate_chat_url("https://chat.mundusx.ai/").is_ok());
        assert!(validate_chat_url("http://localhost:8787").is_ok());
        assert!(validate_chat_url("http://chat.mundusx.ai").is_err());
    }

    #[test]
    fn connector_normalizes_only_allowlisted_runtime_events() {
        assert_eq!(
            normalized_event_type("tool_completed"),
            Some("tool.completed")
        );
        assert_eq!(normalized_event_type("raw_model_transcript"), None);
    }
}
