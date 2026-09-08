use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant};

const CONNECTOR_STALL_TIMEOUT: Duration = Duration::from_secs(180);
static CONNECTOR_ACTIVITY: OnceLock<Mutex<Instant>> = OnceLock::new();

fn record_connector_activity() {
    if let Some(activity) = CONNECTOR_ACTIVITY.get() {
        if let Ok(mut last_activity) = activity.lock() {
            *last_activity = Instant::now();
        }
    }
}

fn connector_stalled(idle: Duration) -> bool {
    idle >= CONNECTOR_STALL_TIMEOUT
}

fn start_connector_watchdog() {
    start_watchdog(CONNECTOR_STALL_TIMEOUT, Duration::from_secs(5));
}

fn start_watchdog(timeout: Duration, interval: Duration) {
    let activity = CONNECTOR_ACTIVITY.get_or_init(|| Mutex::new(Instant::now()));
    thread::spawn(move || loop {
        thread::sleep(interval);
        let stalled = activity
            .lock()
            .map(|last| last.elapsed() >= timeout)
            .unwrap_or(true);
        if stalled {
            // Do not log here: blocked console/file output can itself cause the
            // stall. Exiting lets the tray relaunch with the saved credentials.
            std::process::exit(75);
        }
    });
}

const PROJECT_INITIALIZE_PROMPT: &str = "__mundusx_initialize_project__";
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

fn bootstrap_token(chat_url: &str, device_name: &str, device_id: &str) -> Result<String, String> {
    let bootstrap: Value = ureq::post(&format!("{chat_url}/api/agent/bootstrap/sessions"))
        .send_json(json!({"device_name": device_name, "device_id": device_id}))
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

// This installation identity survives credential rotation and account switches.
// Upgrade in place by adopting the existing connection UUID when available.
fn persistent_device_id(data_dir: &Path) -> Result<String, String> {
    let path = connection_file(data_dir);
    let mut value = fs::read_to_string(&path).ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(id) = value["device_id"].as_str().filter(|id| Uuid::parse_str(id).is_ok()) {
        return Ok(id.to_string());
    }
    let id = value["connection_id"].as_str().filter(|id| Uuid::parse_str(id).is_ok())
        .map(str::to_string).unwrap_or_else(|| Uuid::new_v4().to_string());
    value["device_id"] = json!(id);
    fs::create_dir_all(data_dir).map_err(|error| format!("could not create device directory: {error}"))?;
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap())
        .map_err(|error| format!("could not save device identity: {error}"))?;
    Ok(id)
}

fn save_connection_id(data_dir: &Path, id: &str) -> Result<(), String> {
    Uuid::parse_str(id).map_err(|_| "server returned an invalid connection identity".to_string())?;
    let path = connection_file(data_dir);
    let mut value: Value = serde_json::from_str(&fs::read_to_string(&path)
        .map_err(|e| format!("could not read connection identity: {e}"))?)
        .map_err(|e| format!("could not parse connection identity: {e}"))?;
    value["connection_id"] = json!(id);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap())
        .map_err(|e| format!("could not save connection identity: {e}"))
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
    record_connector_activity();
    let result = ureq::post(&format!("{url}{path}"))
        .timeout(Duration::from_secs(30))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .map_err(|error| format!("Chat connector request failed: {error}"))?
        .into_json()
        .map_err(|error| format!("Chat connector returned invalid JSON: {error}"));
    // Retryable failures still prove that the loop is responsive. Task
    // heartbeats use this same path, so long-running work stays alive.
    record_connector_activity();
    result
}

fn transient_agent_failure(error: &str) -> bool {
    let value = error.to_ascii_lowercase();
    [
        "http 408",
        "http 425",
        "http 429",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
        "status code 408",
        "status code 425",
        "status code 429",
        "status code 500",
        "status code 502",
        "status code 503",
        "status code 504",
        "application failed to respond",
        "timed out",
        "connection reset",
        "connection closed",
        "connection refused",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn requires_project_file_change(prompt: &str) -> bool {
    let value = prompt.to_ascii_lowercase();
    [
        "create",
        "make",
        "add",
        "write",
        "edit",
        "modify",
        "update",
        "delete",
        "remove",
        "rename",
        "move",
        "generate",
        "scaffold",
        "implement",
        "fix",
        "refactor",
        "format",
        "install",
        "build",
    ]
    .iter()
    .any(|word| {
        value
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|token| token == *word)
    })
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

fn structured_hermes_event(item: &Value, sequence: u64) -> Value {
    let raw_type = item["type"].as_str().unwrap_or("agent_progress");
    let event_type = match raw_type {
        "tool_start" | "tool_started" => "tool_started",
        "tool_complete" | "tool_completed" => "tool_completed",
        "model_start" | "model_requested" => "model_requested",
        "model_complete" | "model_completed" => "model_turn_completed",
        "skills_selected" => "skills_selected",
        "skills_unavailable" => "skills_unavailable",
        _ => "agent_progress",
    };
    let tool = item["data"]["tool"]
        .as_str()
        .or_else(|| item["data"]["name"].as_str());
    let skills = item["data"]["skills"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    json!({
        "sequence": sequence,
        "event": {
            "type": event_type,
            "summary": match tool {
                Some(name) if event_type == "tool_started" => format!("Running {name}"),
                Some(name) if event_type == "tool_completed" => format!("Completed {name}"),
                _ if event_type == "skills_selected" && !skills.is_empty() => format!(
                    "Using {}",
                    skills.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                ),
                _ if event_type == "model_requested" => "Asking the model for the next step".to_string(),
                _ if event_type == "model_turn_completed" => "Model step completed".to_string(),
                _ => "Hermes is working".to_string(),
            },
            "metadata": {"tool": tool, "skills": skills, "source_type": raw_type}
        }
    })
}

fn request_requires_verification(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    let words = lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    words
        .iter()
        .any(|word| matches!(*word, "test" | "tests" | "build" | "compile" | "lint"))
        || lower.contains("run them")
        || lower.contains("run it")
}

fn successful_verification(response: &Value) -> bool {
    response["events"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|event| {
            matches!(
                event["type"].as_str(),
                Some("tool_complete" | "tool_completed")
            ) && event["data"]["verification"].as_bool() == Some(true)
                && event["data"]["success"].as_bool() == Some(true)
        })
}

fn harness_completion_content(
    content: &str,
    changed_files: &[String],
    skills: &[Value],
    verified: bool,
) -> String {
    let mut sections = vec![content.trim().to_string()];
    let skill_names = skills.iter().filter_map(Value::as_str).collect::<Vec<_>>();
    if !skill_names.is_empty() {
        sections.push(format!("Skills used: {}", skill_names.join(", ")));
    }
    if !changed_files.is_empty() {
        sections.push(format!(
            "Files changed:\n{}",
            changed_files
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if verified {
        sections.push("Verification: passed".to_string());
    }
    sections
        .into_iter()
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn post_task_events(
    options: &ConnectorOptions,
    task_id: &str,
    events: Vec<Value>,
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    post_remote(
        &options.chat_url,
        &options.token,
        &format!("/api/agent/connector/tasks/{task_id}/events"),
        json!({"events": events}),
    )
    .map(|_| ())
}

fn workspace_snapshot(root: &Path) -> BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)> {
    fn visit(
        root: &Path,
        current: &Path,
        result: &mut BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)>,
    ) {
        if result.len() >= 5_000 {
            return;
        }
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            if result.len() >= 5_000 {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            if matches!(
                name.to_str(),
                Some(".git" | "node_modules" | "target" | ".venv")
            ) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                visit(root, &path, result);
            } else if metadata.is_file() {
                if let Ok(relative) = path.strip_prefix(root) {
                    result.insert(
                        relative.to_path_buf(),
                        (metadata.len(), metadata.modified().ok()),
                    );
                }
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn changed_file_events(
    before: &BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)>,
    after: &BTreeMap<PathBuf, (u64, Option<std::time::SystemTime>)>,
) -> Vec<Value> {
    before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .take(200)
        .enumerate()
        .map(|(index, path)| {
            let action = if !before.contains_key(path) {
                "created"
            } else if !after.contains_key(path) {
                "deleted"
            } else {
                "modified"
            };
            json!({
                "sequence": 10_000 + index,
                "event": {
                    "type": "file_changed",
                    "summary": format!("{} {}", action, path.display()),
                    "metadata": {"path": path.display().to_string(), "action": action}
                }
            })
        })
        .collect()
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

    // Project creation is a filesystem operation, not a model turn.  Keeping it
    // in the connector protocol makes the browser wait for proof that the
    // directory exists instead of optimistically creating browser-only state.
    if prompt == PROJECT_INITIALIZE_PROMPT {
        post_task_events(
            options,
            &task_id,
            vec![json!({
                "sequence": 1,
                "event": {
                    "type": "project_initialized",
                    "summary": "Project folder created on this computer",
                    "metadata": {"workspace": task_workspace.display().to_string()}
                }
            })],
        )?;
        post_remote(
            &options.chat_url,
            &options.token,
            &format!("/api/agent/connector/tasks/{task_id}/complete"),
            json!({"status": "completed", "result": {
                "content": "Project folder is ready",
                "session_id": session_id,
                "workspace": task_workspace.display().to_string(),
                "changed_files": [],
                "skills": [],
                "verified": true
            }}),
        )?;
        return Ok(());
    }
    let before_files = workspace_snapshot(&task_workspace);
    post_task_events(
        options,
        &task_id,
        vec![
            json!({
                "sequence": 1,
                "event": {
                    "type": "harness_started",
                    "summary": format!("{} started in the project workspace", if runtime == "hermes" { "Hermes" } else { "MundusX Local" }),
                    "metadata": {"runtime": runtime}
                }
            }),
            json!({
                "sequence": 2,
                "event": {
                    "type": "model_turn_queued",
                    "summary": "Planning the project work",
                    "metadata": {"runtime": runtime}
                }
            }),
        ],
    )?;
    let bounded_prompt = workspace_relative
        .map(|relative| {
            if runtime == "hermes" {
                format!(
                    "The current directory is the complete project boundary. Treat every explicit user requirement as an acceptance criterion. Before giving a final response, inspect the resulting files and run the applicable tests or executable command. Never claim success for a check you did not run.\n\n{prompt}"
                )
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

    let mut streamed_event_sequence = 1_000_u64;
    let mut stream_hermes_event = |event: Value| {
        let mapped = structured_hermes_event(&event, streamed_event_sequence);
        streamed_event_sequence += 1;
        let _ = post_task_events(options, &task_id, vec![mapped]);
    };
    let mut run_hermes_with_recovery = |initial_prompt: &str| {
        const HARNESS_RECOVERY_ATTEMPTS: usize = 6;
        let mut resume_prompt = initial_prompt.to_string();
        let mut last_error = String::new();
        for attempt in 0..HARNESS_RECOVERY_ATTEMPTS {
            match super::hermes_adapter::run(
                &resume_prompt,
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
                Some(&mut stream_hermes_event),
            ) {
                Ok(value) => return Ok(value),
                Err(error)
                    if super::hermes_adapter::is_retryable_model_failure(&error)
                        && attempt + 1 < HARNESS_RECOVERY_ATTEMPTS
                        && !stop.load(Ordering::Relaxed) =>
                {
                    last_error = error;
                    // A failed model turn may leave Hermes' provider transcript
                    // over-sized or malformed. Reusing that same transcript makes
                    // every process-level retry deterministic. Keep the project
                    // files, but rebuild agent context from the original request.
                    super::hermes_adapter::clear_session(&super::data_dir(), &session_id)?;
                    let delay_seconds = (2_u64.pow(attempt as u32)).min(30);
                    let _ = post_task_events(
                        options,
                        &task_id,
                        vec![json!({
                            "sequence": 10_000 + attempt,
                            "event": {
                                "type": "model_turn_recovering",
                                "summary": format!("EHDA interrupted the model turn; Hermes is rebuilding context automatically (attempt {} of {})", attempt + 2, HARNESS_RECOVERY_ATTEMPTS),
                                "metadata": {"attempt": attempt + 2, "max_attempts": HARNESS_RECOVERY_ATTEMPTS, "delay_seconds": delay_seconds}
                            }
                        })],
                    );
                    for _ in 0..delay_seconds {
                        if stop.load(Ordering::Relaxed) {
                            return Err("Hermes task was cancelled".to_string());
                        }
                        thread::sleep(Duration::from_secs(1));
                    }
                    resume_prompt = format!(
                        "Resume the existing Hermes project task after a temporary model-service interruption. Continue from the files and tool results already present in the current project directory. Do not repeat completed work. Inspect current state, finish every acceptance criterion, and run the required verification.\n\nOriginal request:\n{initial_prompt}"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error)
    };
    let mut response = if runtime == "hermes" {
        run_hermes_with_recovery(&bounded_prompt)
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
    if runtime == "hermes"
        && allow_mutations
        && requires_project_file_change(&prompt)
        && response.is_ok()
    {
        let used_tools = response
            .as_ref()
            .ok()
            .and_then(|value| value["tool_calls"].as_array())
            .map(|calls| !calls.is_empty())
            .unwrap_or(false);
        let changed_workspace = workspace_snapshot(&task_workspace) != before_files;
        let verification_required = request_requires_verification(&prompt);
        let verified = response.as_ref().ok().is_some_and(successful_verification);
        if !used_tools || !changed_workspace || (verification_required && !verified) {
            let _ = post_task_events(
                options,
                &task_id,
                vec![json!({
                    "sequence": 50,
                    "event": {
                        "type": "acceptance_retrying",
                        "summary": "Hermes returned without changing the project; continuing with tool execution",
                        "metadata": {"used_tools": used_tools, "changed_workspace": changed_workspace, "verification_required": verification_required, "verified": verified}
                    }
                })],
            );
            let correction = format!(
                "Continue the existing project task now. The previous response did not yet satisfy all acceptance criteria. Use the available coding tools to implement the requested files, inspect them, and run the applicable tests, build, or lint command successfully. Do not return source code only in chat and do not claim a check passed unless its command exited successfully.\n\nOriginal request:\n{prompt}"
            );
            response = run_hermes_with_recovery(&correction);
        }
        if response.is_ok() && workspace_snapshot(&task_workspace) == before_files {
            response = Err(
                "Hermes returned without changing the project; the task was not completed"
                    .to_string(),
            );
        }
        if request_requires_verification(&prompt)
            && response
                .as_ref()
                .ok()
                .is_some_and(|value| !successful_verification(value))
        {
            response = Err(
                "Hermes changed project files but did not complete the requested verification successfully"
                    .to_string(),
            );
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = heartbeat.join();

    let events = sanitized_events(&session_id)
        .into_iter()
        .enumerate()
        .map(|(index, mut event)| {
            event["sequence"] = json!(100 + index);
            event
        })
        .collect::<Vec<_>>();
    if !events.is_empty() {
        let _ = post_task_events(options, &task_id, events);
    }
    let after_files = workspace_snapshot(&task_workspace);
    let changed_events = changed_file_events(&before_files, &after_files);
    let changed_files = changed_events
        .iter()
        .filter_map(|item| item["event"]["metadata"]["path"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !changed_events.is_empty() {
        let _ = post_task_events(options, &task_id, changed_events);
    }
    match response {
        Ok(value) => {
            let content = value["choices"][0]["message"]["content"]
                .as_str()
                .ok_or("local agent response did not contain assistant content")?;
            let skills = value["skills"].as_array().cloned().unwrap_or_default();
            let verified = successful_verification(&value);
            let content = harness_completion_content(content, &changed_files, &skills, verified);
            post_remote(
                &options.chat_url,
                &options.token,
                &format!("/api/agent/connector/tasks/{task_id}/complete"),
                json!({"status": "completed", "result": {
                    "content": content,
                    "session_id": session_id,
                    "changed_files": changed_files,
                    "skills": skills,
                    "verified": verified,
                }}),
            )?;
        }
        Err(error) => {
            post_remote(
                &options.chat_url,
                &options.token,
                &format!("/api/agent/connector/tasks/{task_id}/complete"),
                json!({"status": "failed", "error": error, "result": {
                    "session_id": session_id,
                    "changed_files": changed_files,
                    "partial_changes": !changed_files.is_empty(),
                }}),
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
    let device_id = persistent_device_id(data_dir)?;
    let mut connection_id = connection_id(data_dir, options.reauthorize)?;
    if options.token.trim().is_empty() && !options.reauthorize {
        options.token = saved_token(data_dir, &options.chat_url).unwrap_or_default();
    }
    if options.token.trim().is_empty() {
        options.token = bootstrap_token(&options.chat_url, &options.device_name, &device_id)?;
        save_token(data_dir, &options.chat_url, &options.token)?;
    }
    start_connector_watchdog();
    let selected = super::selected_agent();
    if matches!(selected, super::AgentSelection::None) {
        return Err("no local agent is selected; choose one with `mundusx agent use native` or `mundusx agent use hermes` before connecting".to_string());
    }
    let mut runtimes = vec!["native"];
    if super::hermes_adapter::available() {
        runtimes.push("hermes");
    }
    let registration = post_remote(
        &options.chat_url,
        &options.token,
        "/api/agent/connector/register",
        json!({
            "connection_id": connection_id,
            "device_id": device_id,
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
    if let Some(canonical) = registration["connection_id"].as_str() {
        save_connection_id(data_dir, canonical)?;
        connection_id = canonical.to_string();
    }
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
                    // Failures during workspace validation happen before
                    // run_task's normal completion path. Report them so the
                    // server does not lease and reclaim the same poisoned task
                    // forever while Chat keeps displaying "working".
                    if let Some(task_id) = payload["task"]["task_id"].as_str() {
                        let _ = post_remote(
                            &options.chat_url,
                            &options.token,
                            &format!("/api/agent/connector/tasks/{task_id}/complete"),
                            json!({"status": "failed", "error": error}),
                        );
                    }
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
    use super::{
        bounded_task_workspace, changed_file_events, connection_id, harness_completion_content,
        request_requires_verification, requires_project_file_change, structured_hermes_event,
        successful_verification, transient_agent_failure, validate_chat_url, workspace_snapshot,
    };
    use std::fs;

    #[test]
    fn watchdog_allows_normal_requests_and_retries_but_detects_stalls() {
        assert!(!super::connector_stalled(std::time::Duration::from_secs(
            35
        )));
        assert!(!super::connector_stalled(std::time::Duration::from_secs(
            179
        )));
        assert!(super::connector_stalled(std::time::Duration::from_secs(
            180
        )));
        assert!(super::connector_stalled(std::time::Duration::from_secs(
            600
        )));
    }

    #[test]
    fn watchdog_exits_stalled_process_but_preserves_active_heartbeats() {
        const MODE: &str = "MUNDUSX_WATCHDOG_TEST_MODE";
        if let Ok(mode) = std::env::var(MODE) {
            super::start_watchdog(
                std::time::Duration::from_millis(200),
                std::time::Duration::from_millis(10),
            );
            if mode == "active" {
                for _ in 0..50 {
                    super::record_connector_activity();
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                std::process::exit(0);
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
            panic!("stalled connector was not stopped");
        }
        for (mode, expected) in [("stalled", 75), ("active", 0)] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "chat_connector::tests::watchdog_exits_stalled_process_but_preserves_active_heartbeats"])
                .env(MODE, mode).status().unwrap();
            assert_eq!(status.code(), Some(expected));
        }
    }

    #[test]
    fn connector_requires_secure_remote_transport() {
        assert!(validate_chat_url("https://chat.mundusx.ai/").is_ok());
        assert!(validate_chat_url("http://localhost:8787").is_ok());
        assert!(validate_chat_url("http://chat.mundusx.ai").is_err());
    }

    #[test]
    fn retries_only_transient_agent_failures() {
        assert!(transient_agent_failure(
            "HTTP 502: Application failed to respond"
        ));
        assert!(transient_agent_failure(
            "Chat connector request failed: timed out"
        ));
        assert!(!transient_agent_failure(
            "Hermes rejected an invalid tool argument"
        ));
    }

    #[test]
    fn file_change_acceptance_excludes_read_only_project_commands() {
        assert!(requires_project_file_change(
            "create a Node.js Fibonacci CLI"
        ));
        assert!(requires_project_file_change("fix the failing tests"));
        assert!(!requires_project_file_change("run the existing tests"));
        assert!(!requires_project_file_change(
            "explain how this module works"
        ));
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
    fn device_identity_survives_reauthorization_and_adopts_existing_installation() {
        use super::{persistent_device_id, save_token, save_connection_id, saved_token};
        use uuid::Uuid;
        let root = std::env::temp_dir().join(format!("mundusx-device-{}", Uuid::new_v4()));
        let original = connection_id(&root, false).unwrap();
        let device = persistent_device_id(&root).unwrap();
        assert_eq!(device, original);
        save_token(&root, "https://chat.mundusx.ai", "test-only-credential").unwrap();
        let new_connection = connection_id(&root, true).unwrap();
        assert_ne!(new_connection, original);
        assert_eq!(persistent_device_id(&root).unwrap(), device);
        save_connection_id(&root, &original).unwrap();
        assert_eq!(connection_id(&root, false).unwrap(), original);
        assert_eq!(persistent_device_id(&root).unwrap(), device);
        assert_eq!(saved_token(&root, "https://chat.mundusx.ai").as_deref(), Some("test-only-credential"));
        fs::remove_dir_all(root).unwrap();
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

    #[test]
    fn project_progress_reports_created_files_without_reading_their_contents() {
        let root = std::env::temp_dir().join(format!("mundusx-progress-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).expect("test root");
        let before = workspace_snapshot(&root);
        fs::write(root.join("main.rs"), "fn main() {}\n").expect("write project file");
        let events = changed_file_events(&before, &workspace_snapshot(&root));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"]["type"], "file_changed");
        assert_eq!(events[0]["event"]["metadata"]["path"], "main.rs");
        assert_eq!(events[0]["event"]["metadata"]["action"], "created");
        fs::remove_dir_all(root).expect("remove test root");
    }

    #[test]
    fn hermes_skill_events_are_presented_as_harness_progress() {
        let event = structured_hermes_event(
            &serde_json::json!({
                "type": "skills_selected",
                "data": {"skills": ["test-driven-development"]}
            }),
            42,
        );
        assert_eq!(event["sequence"], 42);
        assert_eq!(event["event"]["type"], "skills_selected");
        assert_eq!(event["event"]["summary"], "Using test-driven-development");
    }

    #[test]
    fn requested_tests_require_a_successful_hermes_verification_event() {
        assert!(request_requires_verification("Add tests and run them"));
        assert!(!request_requires_verification("Create a README"));
        assert!(!request_requires_verification("Use the latest package"));
        assert!(successful_verification(&serde_json::json!({
            "events": [{
                "type": "tool_completed",
                "data": {"name": "terminal", "verification": true, "success": true}
            }]
        })));
        assert!(!successful_verification(&serde_json::json!({
            "events": [{
                "type": "tool_completed",
                "data": {"name": "terminal", "verification": true, "success": false}
            }]
        })));
    }

    #[test]
    fn completed_harness_response_summarizes_evidence_without_file_contents() {
        let summary = harness_completion_content(
            "Implemented the CLI.",
            &["index.js".to_string(), "test.js".to_string()],
            &[serde_json::json!("test-driven-development")],
            true,
        );
        assert!(summary.contains("Skills used: test-driven-development"));
        assert!(summary.contains("- index.js"));
        assert!(summary.contains("Verification: passed"));
    }
}
