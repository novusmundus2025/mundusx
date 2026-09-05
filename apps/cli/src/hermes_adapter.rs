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
    remote_model: Option<(&str, &str)>,
) -> Result<Value, String> {
    if !available() {
        return Err(
            "Hermes runtime is not installed; run `mundusx agent install hermes`".to_string(),
        );
    }
    let usage_path = data_dir.join(format!("hermes-usage-{}.json", Uuid::new_v4()));
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let mut command = Command::new(executable());
    command
        .current_dir(workspace)
        .arg("-z")
        .arg(prompt)
        // Hermes exposes the toolset option as `-t`; it does not support the
        // legacy `--max-turns` option. Passing that option made its value
        // (`100`) get parsed as a positional command.
        .args(["-t", "coding", "--usage-file"])
        .arg(&usage_path);
    // If the MundusX node has an active model, Hermes consumes its authenticated,
    // loopback-only raw inference API. Otherwise Hermes keeps its own configured
    // cloud provider. This preserves MundusX model ownership without nesting the
    // native MundusX agent loop inside Hermes.
    if let Some((base_url, token)) = remote_model {
        command
            .args(["--provider", "custom", "-m", "mundusx-agnostic"])
            .env("OPENAI_BASE_URL", base_url)
            .env("OPENAI_API_KEY", token);
    } else if let Some((model, token, base_url)) = mundusx_local_model() {
        command
            .args(["--provider", "custom", "-m", &model])
            .env("OPENAI_BASE_URL", format!("http://{base_url}/local/v1"))
            .env("OPENAI_API_KEY", token);
    }
    if let Some(hermes_id) = session_map(data_dir).get(mundusx_session_id) {
        command.args(["--resume", hermes_id]);
    }
    // This is only enabled after the MundusX caller explicitly grants mutation authority.
    // Without it, Hermes keeps its own approval gate and non-interactive mutations fail closed.
    if approve_mutations {
        command.arg("--yolo");
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
    if !status.success() {
        let error = String::from_utf8_lossy(&stderr).trim().to_string();
        return Err(if error.is_empty() {
            format!("Hermes exited with {status}")
        } else {
            format!("Hermes failed: {error}")
        });
    }
    Ok(serde_json::json!({
        "choices": [{"message": {"role": "assistant", "content": String::from_utf8_lossy(&stdout).trim()}}],
        "runtime": "hermes",
        "runtime_session_id": usage["session_id"],
        "usage": usage
    }))
}
