use clap::{Parser, Subcommand};
use mundusx_agent_core::{AgentEventStore, SessionId, SqliteEventStore};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

mod chat_connector;

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
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        approve_mutations: bool,
    },
    /// Continue an existing agent session
    Resume {
        session_id: String,
        prompt: String,
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
    /// Connect this local agent to chat.mundusx.ai using an MCP connection token
    Connect {
        #[arg(long, default_value = "https://chat.mundusx.ai")]
        url: String,
        #[arg(long, env = "MUNDUSX_CHAT_TOKEN", hide_env_values = true)]
        token: String,
        #[arg(long)]
        device_name: Option<String>,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
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

fn main() {
    let result = match Cli::parse().command {
        Commands::Run {
            prompt,
            session,
            approve_mutations,
        } => {
            if let Some(value) = session.as_deref() {
                if SessionId::parse(value).is_err() {
                    Err("--session must be a valid MundusX session id".to_string())
                } else {
                    run_prompt(&prompt, Some(value), approve_mutations)
                }
            } else {
                run_prompt(&prompt, None, approve_mutations)
            }
        }
        Commands::Resume {
            session_id,
            prompt,
            approve_mutations,
        } => {
            if SessionId::parse(&session_id).is_err() {
                Err("session_id must be a valid MundusX session id".to_string())
            } else {
                run_prompt(&prompt, Some(&session_id), approve_mutations)
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
                        token,
                        device_name: device_name.unwrap_or_else(|| {
                            std::env::var("COMPUTERNAME")
                                .or_else(|_| std::env::var("HOSTNAME"))
                                .unwrap_or_else(|_| "MundusX agent".to_string())
                        }),
                        workspace,
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
