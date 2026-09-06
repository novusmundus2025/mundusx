use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

const STRUCTURED_BRIDGE: &str = include_str!("hermes_bridge.py");

fn executable() -> PathBuf {
    if let Some(path) = std::env::var_os("MUNDUSX_HERMES_BIN").map(PathBuf::from) {
        return path;
    }
    let binary = if cfg!(windows) {
        "hermes.exe"
    } else {
        "hermes"
    };
    let managed = std::env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".hermes")))
        .map(|home| home.join("bin").join(binary));
    managed
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(binary))
}

fn hermes_home() -> Option<PathBuf> {
    std::env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".hermes")))
}

fn embedded_runtime() -> Option<(PathBuf, PathBuf)> {
    let project = hermes_home()?.join("hermes-agent");
    let python = if cfg!(windows) {
        project.join("venv").join("Scripts").join("python.exe")
    } else {
        project.join("venv").join("bin").join("python")
    };
    (project.is_dir() && python.is_file()).then_some((project, python))
}

fn opengpu_home() -> PathBuf {
    std::env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn local_api_address() -> String {
    std::env::var("OPENGPU_LOCAL_AGENT_ADDR")
        .ok()
        .filter(|value| value.starts_with("127.0.0.1:") || value.starts_with("[::1]:"))
        .unwrap_or_else(|| "127.0.0.1:11435".to_string())
}

fn mundusx_local_model() -> Option<(String, String, String)> {
    let home = opengpu_home();
    let token = fs::read_to_string(home.join("local-agent-token")).ok()?;
    let token = token.trim().to_string();
    if token.len() < 32 {
        return None;
    }
    let config: Value = serde_json::from_slice(&fs::read(home.join("config.json")).ok()?).ok()?;
    let model = config["active_model"].as_str()?.trim().to_string();
    if model.is_empty() {
        return None;
    }
    let address = local_api_address();
    ureq::get(&format!("http://{address}/local/v1/health"))
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(2))
        .call()
        .ok()?;
    Some((model, token, address))
}

pub fn available() -> bool {
    Command::new(executable())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn map_path(data_dir: &Path) -> PathBuf {
    data_dir.join("hermes-sessions.json")
}

fn session_map(data_dir: &Path) -> BTreeMap<String, String> {
    fs::read(map_path(data_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_session(data_dir: &Path, mundusx_id: &str, hermes_id: &str) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let mut sessions = session_map(data_dir);
    sessions.insert(mundusx_id.to_string(), hermes_id.to_string());
    fs::write(
        map_path(data_dir),
        serde_json::to_vec_pretty(&sessions).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("could not save Hermes session mapping: {error}"))
}

pub fn run(
    prompt: &str,
    mundusx_session_id: &str,
    workspace: &Path,
    data_dir: &Path,
    approve_mutations: bool,
    cancellation: Option<&AtomicBool>,
    remote_model: Option<(&str, &str, &str, &str)>,
) -> Result<Value, String> {
    if !available() {
        return Err(
            "Hermes runtime is not installed; run `mundusx agent install hermes`".to_string(),
        );
    }
    let usage_path = data_dir.join(format!("hermes-usage-{}.json", Uuid::new_v4()));
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    // If the MundusX node has an active model, Hermes consumes its authenticated,
    // loopback-only raw inference API. Otherwise Hermes keeps its own configured
    // cloud provider. This preserves MundusX model ownership without nesting the
    // native MundusX agent loop inside Hermes.
    let mut _model_proxy = None;
    let model_runtime =
        if let Some((base_url, token, project_task_id, connection_id)) = remote_model {
            let proxy = super::project_model_proxy::ProjectModelProxy::start(
                base_url,
                token,
                project_task_id,
                connection_id,
            )?;
            let runtime = (
                proxy.base_url(),
                proxy.credential().to_string(),
                "mundusx-agnostic".to_string(),
            );
            _model_proxy = Some(proxy);
            Some(runtime)
        } else if let Some((model, token, base_url)) = mundusx_local_model() {
            Some((format!("http://{base_url}/local/v1"), token, model))
        } else {
            None
        };
    let saved_session = session_map(data_dir).get(mundusx_session_id).cloned();
    let mut structured = false;
    let mut command = if let (Some((project, python)), Some((base_url, token, _model))) =
        (embedded_runtime(), model_runtime.as_ref())
    {
        structured = true;
        let bridge_path = data_dir.join("hermes-bridge.py");
        fs::write(&bridge_path, STRUCTURED_BRIDGE)
            .map_err(|error| format!("could not prepare structured Hermes bridge: {error}"))?;
        let mut command = Command::new(python);
        command
            .arg(bridge_path)
            .env("MUNDUSX_HERMES_PROJECT_ROOT", project)
            .env("MUNDUSX_HERMES_PROMPT", prompt)
            .env("MUNDUSX_HERMES_TASK", mundusx_session_id)
            .env("OPENAI_BASE_URL", base_url)
            .env("OPENAI_API_KEY", token);
        if let Some(session) = saved_session.as_deref() {
            command.env("MUNDUSX_HERMES_SESSION", session);
        }
        command
    } else {
        let mut command = Command::new(executable());
        command
            .arg("-z")
            .arg(prompt)
            // Match the embedded bridge: project tasks need both the coding
            // tools and Hermes' native SKILL.md discovery/loading tools.
            .args(["-t", "coding,skills", "--usage-file"])
            .arg(&usage_path);
        if let Some((base_url, token, model)) = model_runtime.as_ref() {
            command
                .args(["--provider", "openai-api", "-m", model])
                .env("OPENAI_BASE_URL", base_url)
                .env("OPENAI_API_KEY", token);
        }
        if let Some(session) = saved_session.as_deref() {
            command.args(["--resume", session]);
        }
        command
    };
    command.current_dir(workspace);
    // This is only enabled after the MundusX caller explicitly grants mutation authority.
    // Without it, Hermes keeps its own approval gate and non-interactive mutations fail closed.
    if approve_mutations {
        if structured {
            command.env("HERMES_YOLO_MODE", "1");
        } else {
            command.arg("--yolo");
        }
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start Hermes: {error}"))?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or("could not capture Hermes output")?;
    let mut child_stderr = child
        .stderr
        .take()
        .ok_or("could not capture Hermes errors")?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = child_stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = child_stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = loop {
        if cancellation
            .map(|flag| flag.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&usage_path);
            return Err("Hermes task was cancelled".to_string());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not monitor Hermes: {error}"))?
        {
            break status;
        }
        thread::sleep(Duration::from_millis(200));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| "Hermes output reader failed")?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "Hermes error reader failed")?;
    let usage: Value = fs::read(&usage_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let _ = fs::remove_file(&usage_path);
    if let Some(hermes_id) = usage["session_id"].as_str() {
        save_session(data_dir, mundusx_session_id, hermes_id)?;
    }
    let content = String::from_utf8_lossy(&stdout).trim().to_string();
    if structured {
        let events = content
            .lines()
            .filter_map(|line| line.strip_prefix("MUNDUSX_EVENT="))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect::<Vec<_>>();
        let result = content
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix("MUNDUSX_RESULT="))
            .and_then(|line| serde_json::from_str::<Value>(line).ok())
            .ok_or_else(|| {
                let error = String::from_utf8_lossy(&stderr).trim().to_string();
                if error.is_empty() {
                    "structured Hermes bridge returned no result".to_string()
                } else {
                    format!("structured Hermes bridge returned no result: {error}")
                }
            })?;
        if let Some(session) = result["session_id"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            save_session(data_dir, mundusx_session_id, session)?;
        }
        if result["failed"].as_bool().unwrap_or(false)
            || result["partial"].as_bool().unwrap_or(false)
            || !status.success()
        {
            return Err(result["error"]
                .as_str()
                .filter(|value| !value.is_empty())
                .unwrap_or("Hermes did not complete the project task")
                .to_string());
        }
        let final_response = result["final_response"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or("structured Hermes response did not contain final_response")?;
        return Ok(serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": final_response}}],
            "runtime": "hermes",
            "runtime_session_id": result["session_id"],
            "events": events,
            "tool_calls": result["tool_calls"],
            "turn_count": result["turn_count"],
            "skills": result["skills"],
        }));
    }
    if !status.success() {
        let error = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if error.is_empty() {
            format!("Hermes exited with {status}")
        } else {
            format!("Hermes failed: {error}")
        });
    }
    if hermes_output_reports_model_failure(&content) {
        return Err(format!("Hermes model request failed: {content}"));
    }
    Ok(serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": content}}],
        "runtime": "hermes",
        "runtime_session_id": usage["session_id"],
        "usage": usage
    }))
}

fn hermes_output_reports_model_failure(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("api call failed after")
        || lower.contains("application failed to respond")
        || lower.contains("model gateway returned 5")
        || content.lines().any(|line| {
            line.trim_start()
                .strip_prefix("HTTP ")
                .and_then(|rest| rest.get(..3))
                .is_some_and(|code| {
                    code.starts_with('5') && code.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
}

#[cfg(test)]
mod output_tests {
    use super::{hermes_output_reports_model_failure, STRUCTURED_BRIDGE};

    #[test]
    fn structured_bridge_enables_native_hermes_skills() {
        assert!(STRUCTURED_BRIDGE.contains("enabled_toolsets=[\"coding\", \"skills\"]"));
        assert!(STRUCTURED_BRIDGE.contains("build_preloaded_skills_prompt"));
        assert!(STRUCTURED_BRIDGE.contains("skills_selected"));
    }

    #[test]
    fn classifies_retried_gateway_errors_as_failures() {
        assert!(hermes_output_reports_model_failure(
            "API call failed after 3 retries: HTTP 502: model gateway returned 502: Application failed to respond",
        ));
        assert!(hermes_output_reports_model_failure("HTTP 503: unavailable"));
        assert!(!hermes_output_reports_model_failure(
            "Created files and verified the CLI."
        ));
    }
}
