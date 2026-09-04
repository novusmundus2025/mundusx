use clap::{Parser, Subcommand};
use mundusx_agent_core::{AgentEventStore, SessionId, SqliteEventStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod chat_connector;
mod hermes_adapter;

const DEFAULT_AGENT_URL: &str = "http://127.0.0.1:11436";

#[derive(Parser)]
#[command(name = "mundusx", version, about = "Local-first MundusX AI agent")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a task with the local MundusX agent
    Run {
        prompt: String,
        /// Override the configured agent for this run
        #[arg(long, value_enum)]
        runtime: Option<AgentRuntime>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        approve_mutations: bool,
    },
    /// Continue an existing agent session
    Resume {
        session_id: String,
        prompt: String,
        /// Override the configured agent for this turn
        #[arg(long, value_enum)]
        runtime: Option<AgentRuntime>,
        #[arg(long)]
        approve_mutations: bool,
    },
    /// List local agent sessions
    Sessions {
        #[arg(long)]
        json: bool,
    },
    /// Cancel a running agent session between turns
    Cancel { session_id: String },
    /// Run the local OpenAI-compatible agent service
    Serve {
        #[arg(long, default_value = "127.0.0.1:11436")]
        bind: String,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    /// Manage local models (forwarded to the compatibility implementation)
    Model {
        #[arg(required = true, trailing_var_arg = true)]
        arguments: Vec<String>,
    },
    /// Select, install, or inspect the local agent harness
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Manage contributed compute (forwarded to OpenGPU)
    Contributor {
        #[arg(required = true, trailing_var_arg = true)]
        arguments: Vec<String>,
    },
    /// Connect a local project to Chat; opens the browser for approval when needed
    Connect {
        #[arg(long, default_value = "https://chat.mundusx.ai")]
        url: String,
        #[arg(long, env = "MUNDUSX_CHAT_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long)]
        device_name: Option<String>,
        /// Local project root (defaults to ~/MundusX/Projects on every OS)
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
enum AgentRuntime {
    Native,
    Hermes,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
enum AgentSelection {
    Hermes,
    Native,
    None,
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Make this the default harness for local and connected tasks
    Use { agent: AgentSelection },
    /// Install a supported external harness
    Install { agent: AgentSelection },
    /// Remove an external harness integration
    Remove { agent: AgentSelection },
    /// Show the selected and available harnesses
    Status,
}

#[derive(Deserialize, Serialize)]
struct AgentPreferences {
    selected: AgentSelection,
}

fn data_dir() -> PathBuf {
    std::env::var_os("MUNDUSX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".mundusx")))
        .unwrap_or_else(|| PathBuf::from(".mundusx"))
}

fn agent_url() -> String {
    std::env::var("MUNDUSX_AGENT_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_AGENT_URL.to_string())
}

fn api_key() -> Option<String> {
    std::env::var("MUNDUSX_AGENT_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn post_chat(
    prompt: &str,
    session_id: Option<&str>,
    approve_mutations: bool,
) -> Result<Value, String> {
    let mut request = ureq::post(&format!(
        "{}/v1/chat/completions",
        agent_url().trim_end_matches('/')
    ));
    if let Some(key) = api_key() {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }
    if let Some(session_id) = session_id {
        request = request.set("X-MundusX-Session-Id", session_id);
    }
    if approve_mutations {
        request = request.set("X-MundusX-Allow-Mutations", "true");
    }
    request
        .send_json(json!({
            "model": "mundusx-agent",
            "messages": [{"role": "user", "content": prompt}],
            "stream": false
        }))
        .map_err(|error| format!("agent request failed: {error}"))?
        .into_json()
        .map_err(|error| format!("agent returned invalid JSON: {error}"))
}

fn server_executable() -> PathBuf {
    std::env::var_os("MUNDUSX_AGENT_SERVER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| PathBuf::from("."))
                .join(if cfg!(windows) {
                    "mundusx-agent-server.exe"
                } else {
                    "mundusx-agent-server"
                })
        })
}

fn spawn_server(workspace: &Path) -> Result<(), String> {
    let executable = server_executable();
    let mut command = Command::new(&executable);
    command
        .arg("--workspace")
        .arg(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn().map_err(|error| {
        format!(
            "could not start {}: {error}; install the MundusX agent-server component",
            executable.display()
        )
    })?;
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(100));
        if ureq::get(&format!("{}/health", agent_url().trim_end_matches('/')))
            .call()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("MundusX agent server did not become ready".to_string())
}

fn run_prompt(
    prompt: &str,
    session_id: Option<&str>,
    approve_mutations: bool,
) -> Result<(), String> {
    let generated_session;
    let session_id = match session_id {
        Some(value) => value,
        None => {
            generated_session = SessionId::new().to_string();
            &generated_session
        }
    };
    eprintln!("session: {session_id}");
    let response = match post_chat(prompt, Some(session_id), approve_mutations) {
        Ok(value) => value,
        Err(first_error) => {
            let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
            spawn_server(&workspace)
                .map_err(|start_error| format!("{first_error}; {start_error}"))?;
            post_chat(prompt, Some(session_id), approve_mutations)?
        }
    };
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| format!("agent response did not contain assistant content: {response}"))?;
    println!("{content}");
    Ok(())
}

fn preferences_path() -> PathBuf {
    data_dir().join("agent-config.json")
}

fn selected_agent() -> AgentSelection {
    std::fs::read(preferences_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AgentPreferences>(&bytes).ok())
        .map(|config| config.selected)
        .unwrap_or(AgentSelection::Native)
}

fn save_selected_agent(selected: AgentSelection) -> Result<(), String> {
    let directory = data_dir();
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    std::fs::write(
        preferences_path(),
        serde_json::to_vec_pretty(&AgentPreferences { selected })
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("could not save agent preference: {error}"))
}

fn effective_runtime(override_runtime: Option<AgentRuntime>) -> Result<AgentRuntime, String> {
    if let Some(runtime) = override_runtime {
        return Ok(runtime);
    }
    match selected_agent() {
        AgentSelection::Native => Ok(AgentRuntime::Native),
        AgentSelection::Hermes => Ok(AgentRuntime::Hermes),
        AgentSelection::None => Err(
            "no local agent is selected; run `mundusx agent use native` or `mundusx agent use hermes`"
                .to_string(),
        ),
    }
}

fn run_with_runtime(
    runtime: AgentRuntime,
    prompt: &str,
    session_id: Option<&str>,
    approve_mutations: bool,
) -> Result<(), String> {
    if matches!(runtime, AgentRuntime::Native) {
        return run_prompt(prompt, session_id, approve_mutations);
    }
    let generated_session;
    let session_id = match session_id {
        Some(value) => value,
        None => {
            generated_session = SessionId::new().to_string();
            &generated_session
        }
    };
    eprintln!("session: {session_id}");
    let workspace = std::env::current_dir().map_err(|error| error.to_string())?;
    let response = hermes_adapter::run(
        prompt,
        session_id,
        &workspace,
        &data_dir(),
        approve_mutations,
        None,
        None,
    )?;
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("Hermes response did not contain assistant content")?;
    println!("{content}");
    Ok(())
}

fn cancel_session(session_id: &str) -> Result<(), String> {
    SessionId::parse(session_id).map_err(|_| "session_id must be valid".to_string())?;
    let mut request = ureq::post(&format!(
        "{}/v1/sessions/{session_id}/cancel",
        agent_url().trim_end_matches('/')
    ));
    if let Some(key) = api_key() {
        request = request.set("Authorization", &format!("Bearer {key}"));
    }
    request
        .call()
        .map_err(|error| format!("could not cancel session: {error}"))?;
    println!("Cancellation requested for {session_id}");
    Ok(())
}

fn list_sessions(json_output: bool) -> Result<(), String> {
    let store =
        SqliteEventStore::open(data_dir().join("agent.db")).map_err(|error| error.to_string())?;
    let sessions = store.sessions().map_err(|error| error.to_string())?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&sessions).map_err(|error| error.to_string())?
        );
    } else if sessions.is_empty() {
        println!("No MundusX agent sessions yet.");
    } else {
        for session in sessions {
            println!(
                "{}\t{}\t{}",
                session.id,
                session
                    .title
                    .unwrap_or_else(|| "Untitled session".to_string()),
                session.working_directory
            );
        }
    }
    Ok(())
}

fn forward_model(arguments: &[String]) -> Result<(), String> {
    let executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(if cfg!(windows) {
            "opengpu.exe"
        } else {
            "opengpu"
        });
    let status = Command::new(&executable)
        .arg("model")
        .args(arguments)
        .status()
        .map_err(|error| format!("could not run {}: {error}", executable.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("model command exited with {status}"))
    }
}

fn forward_opengpu(arguments: &[String]) -> Result<(), String> {
    let executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(if cfg!(windows) {
            "opengpu.exe"
        } else {
            "opengpu"
        });
    let status = Command::new(&executable)
        .args(arguments)
        .status()
        .map_err(|error| format!("could not run {}: {error}", executable.display()))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("contributor command exited with {status}"))
}

fn install_hermes() -> Result<(), String> {
    if hermes_adapter::available() {
        println!("Hermes Agent is already installed.");
        return Ok(());
    }
    if !cfg!(windows) {
        return Err("automatic Hermes installation is currently supported on Windows only; see https://github.com/NousResearch/Hermes-Agent".to_string());
    }
    let script = r#"$ErrorActionPreference='Stop'; $uri='https://github.com/mundusx/releases/releases/download/opengpu-prod/hermes-install.ps1'; $path=Join-Path $env:TEMP 'mundusx-hermes-install.ps1'; Invoke-WebRequest -Uri $uri -OutFile $path; $expected='226C70A90AD47E8A4D34CB11ACA4ECBEB649E2F9B67FBD009EA49791DE2D56F5'; $actual=(Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash; if ($actual -ne $expected) { throw "Hermes installer checksum mismatch (expected $expected, got $actual)" }; $hermesRoot=Join-Path $env:USERPROFILE '.hermes'; & $path -HermesHome $hermesRoot -InstallDir (Join-Path $hermesRoot 'hermes-agent') -Commit '9de9c25f620ff7f1ce0fd5457d596052d5159596' -SkipSetup; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }"#;
    println!(
        "Hermes setup started. Dependency installation can take several minutes; progress will remain visible."
    );
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .spawn()
        .map_err(|error| format!("could not start the verified Hermes installer: {error}"))?;
    let started = Instant::now();
    let mut next_activity_update = Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not monitor the Hermes installer: {error}"))?
        {
            break status;
        }

        let elapsed = started.elapsed();
        if elapsed >= next_activity_update {
            println!("[Hermes setup] working... {}s elapsed", elapsed.as_secs());
            next_activity_update += Duration::from_secs(5);
        }
        thread::sleep(Duration::from_millis(250));
    };
    if !status.success() {
        return Err(format!("Hermes installer exited with {status}"));
    }
    println!(
        "[Hermes setup] complete in {}s",
        started.elapsed().as_secs()
    );
    save_selected_agent(AgentSelection::Hermes)?;
    println!(
        "Hermes Agent installed and selected. MundusX will use its active local model when the node is running; run `hermes setup` only if you also want a cloud fallback."
    );
    Ok(())
}

fn manage_agent(command: AgentCommands) -> Result<(), String> {
    match command {
        AgentCommands::Use { agent } => {
            if matches!(agent, AgentSelection::Hermes) && !hermes_adapter::available() {
                return Err("Hermes Agent is not installed; run `mundusx agent install hermes` first".to_string());
            }
            save_selected_agent(agent)?;
            println!("Default agent: {}", serde_json::to_value(agent).unwrap().as_str().unwrap());
            Ok(())
        }
        AgentCommands::Install { agent: AgentSelection::Hermes } => install_hermes(),
        AgentCommands::Install { agent: AgentSelection::Native } => {
            save_selected_agent(AgentSelection::Native)?;
            println!("MundusX Agent is included with MundusX and is now selected.");
            Ok(())
        }
        AgentCommands::Install { agent: AgentSelection::None } => {
            Err("`none` is a selection, not an installable agent; run `mundusx agent use none`".to_string())
        }
        AgentCommands::Remove { agent: AgentSelection::Hermes } => {
            Err("MundusX will not delete an independently installed Hermes workspace. Use Hermes' own uninstaller, then run `mundusx agent use native` or `mundusx agent use none`.".to_string())
        }
        AgentCommands::Remove { agent: AgentSelection::Native } => {
            Err("MundusX Agent is a bundled component and cannot be removed separately; select `none` instead".to_string())
        }
        AgentCommands::Remove { agent: AgentSelection::None } => Err("`none` is not installed".to_string()),
        AgentCommands::Status => {
            let selected = serde_json::to_value(selected_agent()).unwrap();
            println!("selected: {}", selected.as_str().unwrap());
            println!("native: available (bundled)");
            println!("hermes: {}", if hermes_adapter::available() { "available" } else { "not installed" });
            println!("models: managed by `mundusx model`");
            println!("compute: managed by `mundusx contributor`");
            Ok(())
        }
    }
}

fn main() {
    let result = match Cli::parse().command {
        Commands::Run {
            prompt,
            runtime,
            session,
            approve_mutations,
        } => {
            if let Some(value) = session.as_deref() {
                if SessionId::parse(value).is_err() {
                    Err("--session must be a valid MundusX session id".to_string())
                } else {
                    effective_runtime(runtime).and_then(|runtime| {
                        run_with_runtime(runtime, &prompt, Some(value), approve_mutations)
                    })
                }
            } else {
                effective_runtime(runtime)
                    .and_then(|runtime| run_with_runtime(runtime, &prompt, None, approve_mutations))
            }
        }
        Commands::Resume {
            session_id,
            prompt,
            runtime,
            approve_mutations,
        } => {
            if SessionId::parse(&session_id).is_err() {
                Err("session_id must be a valid MundusX session id".to_string())
            } else {
                effective_runtime(runtime).and_then(|runtime| {
                    run_with_runtime(runtime, &prompt, Some(&session_id), approve_mutations)
                })
            }
        }
        Commands::Sessions { json } => list_sessions(json),
        Commands::Cancel { session_id } => cancel_session(&session_id),
        Commands::Serve { bind, workspace } => Command::new(server_executable())
            .args(["--bind", &bind, "--workspace"])
            .arg(workspace)
            .status()
            .map_err(|error| error.to_string())
            .and_then(|status| {
                status
                    .success()
                    .then_some(())
                    .ok_or_else(|| status.to_string())
            }),
        Commands::Model { arguments } => forward_model(&arguments),
        Commands::Agent { command } => manage_agent(command),
        Commands::Contributor { arguments } => forward_opengpu(&arguments),
        Commands::Connect {
            url,
            token,
            device_name,
            workspace,
        } => {
            let url = chat_connector::validate_chat_url(&url);
            url.and_then(|chat_url| {
                chat_connector::connect(
                    chat_connector::ConnectorOptions {
                        chat_url,
                        token: token.unwrap_or_default(),
                        device_name: device_name.unwrap_or_else(|| {
                            std::env::var("COMPUTERNAME")
                                .or_else(|_| std::env::var("HOSTNAME"))
                                .unwrap_or_else(|_| "MundusX agent".to_string())
                        }),
                        workspace: workspace.unwrap_or_else(default_workspace_root),
                    },
                    &data_dir(),
                )
            })
        }
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn default_workspace_root() -> PathBuf {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    };
    home.map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MundusX")
        .join("Projects")
}
