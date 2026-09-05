use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

#[cfg(windows)]
struct ConnectorInstance(*mut core::ffi::c_void);

#[cfg(windows)]
impl Drop for ConnectorInstance {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn acquire_connector_instance() -> Result<ConnectorInstance, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
        System::Threading::CreateMutexW,
    };
    let name: Vec<u16> = std::ffi::OsStr::new("Local\\MundusXChatConnector")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err("could not create the local connector lock".to_string());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        return Err("the MundusX Chat connector is already running".to_string());
    }
    Ok(ConnectorInstance(handle))
}

#[cfg(not(windows))]
fn acquire_connector_instance() -> Result<(), String> {
    Ok(())
}

pub struct ConnectorOptions {
    pub chat_url: String,
    pub token: String,
    pub device_name: String,
    pub workspace: PathBuf,
    pub reauthorize: bool,
    pub authorize_only: bool,
}

fn saved_token(data_dir: &Path, chat_url: &str) -> Option<String> {
    serde_json::from_str::<Value>(&fs::read_to_string(connection_file(data_dir)).ok()?)
        .ok()
        .filter(|value| value["chat_url"].as_str() == Some(chat_url))?["token"]
        .as_str()
        .map(str::to_string)
}

fn save_token(data_dir: &Path, chat_url: &str, connector_token: &str) -> Result<(), String> {
    let path = connection_file(data_dir);
    let mut value = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    value["chat_url"] = json!(chat_url);
    value["token"] = json!(connector_token);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap())
        .map_err(|error| format!("could not save Chat connection: {error}"))
}

pub fn saved_workspace(data_dir: &Path) -> Option<PathBuf> {
    serde_json::from_str::<Value>(&fs::read_to_string(connection_file(data_dir)).ok()?).ok()?
        ["workspace"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

fn save_workspace(data_dir: &Path, workspace: &Path) -> Result<(), String> {
    let path = connection_file(data_dir);
    let mut value = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    value["workspace"] = json!(workspace.display().to_string());
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap())
        .map_err(|error| format!("could not save connector workspace: {error}"))
}

fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open browser: {e}"))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open browser: {e}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("could not open browser: {e}"))
    }
}

fn bootstrap_token(chat_url: &str, device_name: &str) -> Result<String, String> {
    let bootstrap: Value = ureq::post(&format!("{chat_url}/api/agent/bootstrap/sessions"))
        .send_json(json!({"device_name": device_name}))
        .map_err(|e| format!("could not start browser approval: {e}"))?
        .into_json()
        .map_err(|e| format!("Chat returned invalid approval data: {e}"))?;
    let session = bootstrap["session_id"]
        .as_str()
        .ok_or("approval session is missing")?;
    let connector_token = bootstrap["connector_token"]
        .as_str()
        .ok_or("connector credential is missing")?
        .to_string();
    let approval_url = bootstrap["approval_url"]
        .as_str()
        .ok_or("approval URL is missing")?;
    eprintln!("Approve this computer in your browser:\n{approval_url}");
    open_browser(approval_url)?;
    for _ in 0..120 {
        let state: Value = ureq::post(&format!(
            "{chat_url}/api/agent/bootstrap/sessions/{session}/status"
        ))
        .send_json(json!({"connector_token": connector_token}))
        .map_err(|e| format!("approval check failed: {e}"))?
        .into_json()
        .map_err(|e| format!("Chat returned invalid approval status: {e}"))?;
        match state["state"].as_str() {
            Some("approved") => return Ok(connector_token),
            Some("expired") => {
                return Err("browser approval expired; run the command again".to_string())
            }
            _ => thread::sleep(Duration::from_secs(2)),
        }
    }
    Err("browser approval timed out; run the command again".to_string())
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

fn connection_id(data_dir: &Path, regenerate: bool) -> Result<String, String> {
    let path = connection_file(data_dir);
    if !regenerate {
        if let Ok(value) = fs::read_to_string(&path) {
            if let Some(id) = serde_json::from_str::<Value>(&value)
                .ok()
                .and_then(|value| value["connection_id"].as_str().map(str::to_string))
                .filter(|id| Uuid::parse_str(id).is_ok())
            {
                return Ok(id);
            }
        }
    }
    let id = Uuid::new_v4().to_string();
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("could not create MundusX data directory: {error}"))?;
    let mut value = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    value["connection_id"] = json!(id);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap())
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
        let event_type = kind.get("type")?.as_str()?;
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

fn bounded_task_workspace(
    workspace: &Path,
    relative: Option<&str>,
    allow_mutations: bool,
) -> Result<PathBuf, String> {
    let root = workspace
        .canonicalize()
        .map_err(|error| format!("connector workspace is unavailable: {error}"))?;
    let Some(relative) = relative.filter(|value| !value.is_empty()) else {
        return Ok(root);
    };
    if !relative
        .chars()
        .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '-')
        || relative.starts_with('-')
        || relative.ends_with('-')
        || relative.contains("--")
    {
        return Err("task workspace must be a project slug".to_string());
    }
    let target = root.join(relative);
    if !target.exists() {
        if !allow_mutations {
            return Err(
                "project directory does not exist and task has no mutation authority".to_string(),
            );
        }
        fs::create_dir(&target)
            .map_err(|error| format!("could not create project directory: {error}"))?;
    }
    let target = target
        .canonicalize()
        .map_err(|error| format!("project workspace is unavailable: {error}"))?;
    if !target.starts_with(&root) {
        return Err("project workspace escaped the connector boundary".to_string());
    }
    Ok(target)
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
    let runtime = task["runtime_selected"].as_str().unwrap_or("native");
    let workspace_relative = task["workspace_relative"].as_str();
    let task_workspace =
        bounded_task_workspace(&options.workspace, workspace_relative, allow_mutations)?;
    let bounded_prompt = workspace_relative
        .map(|relative| {
            if runtime == "hermes" {
                format!("The current directory is the complete project boundary.\n\n{prompt}")
            } else {
                format!("Work only within project directory `{relative}` beneath the connector workspace.\n\n{prompt}")
            }
        })
        .unwrap_or_else(|| prompt.clone());
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
                    heartbeat_stop.store(true, Ordering::Relaxed);
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

    let response = if runtime == "hermes" {
        super::hermes_adapter::run(
            &bounded_prompt,
            &session_id,
            &task_workspace,
            &super::data_dir(),
            allow_mutations,
            Some(stop.as_ref()),
            Some((
                &format!("{}/api/agent/model/v1", options.chat_url),
                &options.token,
                &task_id,
                connection_id,
            )),
        )
    } else {
        match super::post_chat(&bounded_prompt, Some(&session_id), allow_mutations) {
            Ok(value) => Ok(value),
            Err(first_error) => {
                super::spawn_server(&options.workspace)
                    .map_err(|start_error| format!("{first_error}; {start_error}"))?;
                super::post_chat(&bounded_prompt, Some(&session_id), allow_mutations)
            }
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = heartbeat.join();

    let events = sanitized_events(&session_id);
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
            let content = value["choices"][0]["message"]["content"]
                .as_str()
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

pub fn connect(mut options: ConnectorOptions, data_dir: &Path) -> Result<(), String> {
    let _instance = acquire_connector_instance()?;
    fs::create_dir_all(&options.workspace)
        .map_err(|error| format!("could not create connector workspace: {error}"))?;
    save_workspace(data_dir, &options.workspace)?;
    // Fresh browser approval may intentionally bind this installation to a
    // different account. Rotate the device identity so server-side ownership
    // protection does not mistake that authorized rebind for account theft.
    let connection_id = connection_id(data_dir, options.reauthorize)?;
    if options.token.trim().is_empty() && !options.reauthorize {
        options.token = saved_token(data_dir, &options.chat_url).unwrap_or_default();
    }
    if options.token.trim().is_empty() {
        options.token = bootstrap_token(&options.chat_url, &options.device_name)?;
        save_token(data_dir, &options.chat_url, &options.token)?;
    }
    let selected = super::selected_agent();
    if matches!(selected, super::AgentSelection::None) {
        return Err("no local agent is selected; choose one with `mundusx agent use native` or `mundusx agent use hermes` before connecting".to_string());
    }
    let mut runtimes = vec!["native"];
    if super::hermes_adapter::available() {
        runtimes.push("hermes");
    }
    post_remote(
        &options.chat_url,
        &options.token,
        "/api/agent/connector/register",
        json!({
            "connection_id": connection_id,
            "device_name": options.device_name,
            "capabilities": {
                "protocol": "mundusx-agent-bridge/v1",
                "client_version": option_env!("MUNDUSX_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
                "mutations": false,
                "agent_runtimes": runtimes,
                "preferred_agent": serde_json::to_value(selected).unwrap_or_else(|_| json!("native"))
            }
        }),
    )?;
    if options.authorize_only {
        eprintln!("MundusX Chat connection approved. The background app will keep it online.");
        return Ok(());
    }
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
    use super::{bounded_task_workspace, connection_id, validate_chat_url};
    use std::fs;

    #[test]
    fn connector_requires_secure_remote_transport() {
        assert!(validate_chat_url("https://chat.mundusx.ai/").is_ok());
        assert!(validate_chat_url("http://localhost:8787").is_ok());
        assert!(validate_chat_url("http://chat.mundusx.ai").is_err());
    }

    #[test]
    fn task_workspace_cannot_escape_connector_root() {
        let root = std::env::temp_dir().join(format!("mundusx-boundary-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).expect("test root");
        let project = bounded_task_workspace(&root, Some("my-project"), true).expect("project");
        assert!(project.starts_with(root.canonicalize().expect("canonical root")));
        assert!(bounded_task_workspace(&root, Some("../escape"), true).is_err());
        assert!(bounded_task_workspace(&root, Some("Bad-Name"), true).is_err());
        fs::remove_dir_all(root).expect("remove test root");
    }

    #[test]
    fn reauthorization_rotates_the_account_bound_device_identity() {
        let root = std::env::temp_dir().join(format!("mundusx-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).expect("test root");
        let first = connection_id(&root, false).expect("initial identity");
        assert_eq!(connection_id(&root, false).expect("saved identity"), first);
        let rotated = connection_id(&root, true).expect("rotated identity");
        assert_ne!(rotated, first);
        assert_eq!(
            connection_id(&root, false).expect("new saved identity"),
            rotated
        );
        fs::remove_dir_all(root).expect("remove test root");
    }
}
