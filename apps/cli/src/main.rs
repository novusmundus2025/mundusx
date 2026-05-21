mod config;
mod identity;
mod nodes;
mod routing;
mod types;

use clap::{Parser, Subcommand};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::thread;
use types::{Backend, JobRequest};

use config::{
    config_dir, config_exists, config_path, load_config, remove_config_files, resolved_config_path,
    save_config, Config,
};
use identity::{
    device_id_for_identity, ensure_identity, load_identity, load_or_create_identity,
    resolved_identity_path,
};
use nodes::{live_nodes, sample_nodes};
use routing::select_best_node;

#[derive(Parser, Debug)]
#[command(
    name = "opengpu",
    version,
    about = "OpenGPU CLI",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Init {
        #[arg(long)]
        force: bool,
    },
    Start {
        #[arg(long)]
        m: bool,
        #[arg(long)]
        cuda: bool,
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
        percent: Option<u8>,
    },
    Login {
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    Logout,
    Connect,
    Disconnect,
    Status {
        #[arg(long)]
        json: bool,
    },
    Nodes {
        #[arg(long)]
        json: bool,
    },
    Contribute {
        #[arg(long)]
        m: bool,
        #[arg(long)]
        cuda: bool,
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
        percent: Option<u8>,
    },
    Pause,
    Resume,
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Doctor,
    Logs,
    Update,
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    Path,
    Show {
        #[arg(long)]
        json: bool,
    },
    Set {
        key: ConfigKey,
        value: String,
    },
    Reset {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ConfigKey {
    ControlPlaneUrl,
    ProfileName,
    Backend,
    DeviceId,
    ContributionPercent,
}

fn default_job_request(preferred_backend: Backend) -> JobRequest {
    JobRequest {
        request_id: format!("local-{}", uuid::Uuid::new_v4().simple()),
        prompt: "status".to_string(),
        preferred_backend,
    }
}

fn current_config_or_default() -> Config {
    load_config().ok().flatten().unwrap_or_default()
}

fn display_public_key_fingerprint(config: &Config) -> String {
    config
        .public_key_fingerprint
        .clone()
        .or_else(|| {
            load_identity()
                .ok()
                .flatten()
                .map(|identity| identity.fingerprint)
        })
        .unwrap_or_else(|| "unset".to_string())
}

fn config_from_identity(identity: &identity::DeviceIdentity) -> Config {
    Config {
        device_id: device_id_for_identity(identity),
        public_key_fingerprint: Some(identity.fingerprint.clone()),
        ..Config::default()
    }
}

fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    serde_json::to_string_pretty(value)
        .map(|output| {
            println!("{output}");
        })
        .map_err(|error| error.to_string())
}

fn print_config_summary(config: &Config, path: &std::path::Path) {
    println!("configPath: {}", path.display());
    println!("deviceId: {}", config.device_id);
    println!("publicKeyFingerprint: {}", display_public_key_fingerprint(config));
    println!(
        "profileName: {}",
        config.profile_name.as_deref().unwrap_or("unset")
    );
    println!(
        "authenticated: {}",
        if config.auth_token.is_some() {
            "yes"
        } else {
            "no"
        }
    );
    println!("connected: {}", if config.connected { "yes" } else { "no" });
    println!("paused: {}", if config.paused { "yes" } else { "no" });
    println!("backendPreference: {}", config.backend_preference);
    println!("contributionPercent: {}", config.contribution_percent);
    println!("controlPlaneUrl: {}", config.control_plane_url);
}

fn print_nodes_table() {
    let nodes = sample_nodes();
    let live_count = live_nodes().len();
    println!("liveNodes: {live_count}");
    println!("totalNodes: {}", nodes.len());
    for node in nodes {
        println!(
            "- {} | {} | {} | {} MB | {}% | {}",
            node.node_id,
            node.backend,
            node.state,
            node.available_memory_mb,
            node.available_gpu_percent,
            node.label
        );
    }
}

fn print_startup_summary(config: &Config, path: &std::path::Path) {
    let cores = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);

    println!("startup ready for {}", config.device_id);
    println!("publicKeyFingerprint: {}", display_public_key_fingerprint(config));
    println!("platform: {}-{}", env::consts::OS, env::consts::ARCH);
    println!("cpuCores: {}", cores);
    println!("backendPreference: {}", config.backend_preference);
    println!("contributionPercent: {}", config.contribution_percent);
    println!("connected: yes");
    println!("paused: no");
    println!("configPath: {}", path.display());
}

fn contribution_semantics(backend: Backend) -> &'static str {
    match backend {
        Backend::M => "memory-and-compute budget for Apple Silicon M-series",
        Backend::Cuda => "gpu-utilization budget for CUDA nodes",
        Backend::Auto => "automatic routing budget",
    }
}

fn print_doctor() -> Result<(), String> {
    let primary_dir = config_dir();
    let fallback_dir = PathBuf::from(".opengpu");
    let primary_writable = probe_directory(&primary_dir);
    let fallback_writable = probe_directory(&fallback_dir);

    if !primary_writable && !fallback_writable {
        return Err(format!(
            "cannot write to {} or {}",
            primary_dir.display(),
            fallback_dir.display()
        ));
    }

    println!("configDir: {}", primary_dir.display());
    println!("identityPath: {}", resolved_identity_path().display());
    println!("effectiveConfigPath: {}", resolved_config_path().display());
    println!(
        "fallbackConfigPath: {}",
        config::local_config_path().display()
    );
    println!(
        "primaryWritable: {}",
        if primary_writable { "yes" } else { "no" }
    );
    println!(
        "fallbackWritable: {}",
        if fallback_writable { "yes" } else { "no" }
    );
    match identity::load_identity() {
        Ok(Some(identity)) => {
            println!("deviceIdentity: reused");
            println!("publicKeyFingerprint: {}", identity.fingerprint);
        }
        Ok(None) => println!("deviceIdentity: missing"),
        Err(error) => println!("deviceIdentityError: {error}"),
    }
    println!("installer: https://novusx.ai/install");
    println!("releaseChannel: cli-v*");
    println!("binaryName: opengpu");
    println!("cliVersion: {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

fn probe_directory(dir: &std::path::Path) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }

    let probe = dir.join(".write-test");
    let writable = fs::write(&probe, "ok").is_ok();
    let _ = fs::remove_file(&probe);
    writable
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { force } => {
            let (identity, created, identity_path) = match ensure_identity() {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("failed to initialize identity: {error}");
                    std::process::exit(1);
                }
            };

            if config_exists() && !force {
                if created {
                    println!("generated device identity at {}", identity_path.display());
                } else {
                    println!("reused device identity at {}", identity_path.display());
                }
                println!("config already exists at {}", config_path().display());
                return;
            }

            let config = config_from_identity(&identity);
            match save_config(&config) {
                Ok(path) => {
                    if created {
                        println!("generated device identity at {}", identity_path.display());
                    } else {
                        println!("reused device identity at {}", identity_path.display());
                    }
                    println!("initialized {}", path.display());
                }
                Err(error) => {
                    eprintln!("failed to initialize config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Start { m, cuda, percent } => {
            let (identity, created, identity_path) = match ensure_identity() {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("failed to prepare device identity: {error}");
                    std::process::exit(1);
                }
            };

            if !config_exists() {
                let config = config_from_identity(&identity);
                match save_config(&config) {
                    Ok(path) => {
                        if created {
                            println!("generated device identity at {}", identity_path.display());
                        } else {
                            println!("reused device identity at {}", identity_path.display());
                        }
                        println!("initialized {}", path.display());
                    }
                    Err(error) => {
                        eprintln!("failed to initialize config: {error}");
                        std::process::exit(1);
                    }
                }
            }

            let mut config = current_config_or_default();
            config.device_id = device_id_for_identity(&identity);
            config.public_key_fingerprint = Some(identity.fingerprint.clone());
            config.connected = true;
            config.paused = false;

            if m && cuda {
                eprintln!("choose only one backend: --m or --cuda");
                std::process::exit(1);
            }

            if m {
                config.backend_preference = Backend::M;
            } else if cuda {
                config.backend_preference = Backend::Cuda;
            }

            if let Some(percent) = percent {
                config.contribution_percent = percent;
            }

            match save_config(&config) {
                Ok(path) => {
                    print_startup_summary(&config, &path);
                    println!(
                        "contributionMeaning: {}",
                        contribution_semantics(config.backend_preference)
                    );
                    println!("config saved at {}", path.display());
                }
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Login { token, name } => {
            let mut config = current_config_or_default();
            config.profile_name = name.or(config.profile_name);
            config.auth_token =
                Some(token.unwrap_or_else(|| format!("dev-{}", uuid::Uuid::new_v4().simple())));

            match save_config(&config) {
                Ok(path) => {
                    println!(
                        "authenticated profile {}",
                        config.profile_name.as_deref().unwrap_or("local-user")
                    );
                    println!("config saved at {}", path.display());
                }
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Logout => {
            let mut config = current_config_or_default();
            config.auth_token = None;
            config.connected = false;

            match save_config(&config) {
                Ok(path) => println!("logged out and saved config at {}", path.display()),
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Connect => {
            if !config_exists() {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            if let Ok((identity, _, _)) = load_or_create_identity() {
                config.device_id = device_id_for_identity(&identity);
                config.public_key_fingerprint = Some(identity.fingerprint);
            }
            config.connected = true;
            config.paused = false;

            match save_config(&config) {
                Ok(path) => println!(
                    "connected device {} (config: {})",
                    config.device_id,
                    path.display()
                ),
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Disconnect => {
            if !config_exists() {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            config.connected = false;

            match save_config(&config) {
                Ok(path) => println!(
                    "disconnected device {} (config: {})",
                    config.device_id,
                    path.display()
                ),
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Status { json } => {
            if !config_exists() {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            let config = current_config_or_default();
            let nodes = live_nodes();
            let request = default_job_request(config.backend_preference);
            let decision = select_best_node(&nodes, &request);

            if json {
                let payload = serde_json::json!({
                    "config": config,
                    "live_nodes": nodes,
                    "decision": decision,
                });
                if let Err(error) = print_json(&payload) {
                    eprintln!("failed to print json: {error}");
                    std::process::exit(1);
                }
                return;
            }

            print_config_summary(&config, &resolved_config_path());
            println!("liveNodes: {}", nodes.len());
            println!("routingDecision: {}", decision.reason);
            println!(
                "bestLiveNode: {}",
                decision
                    .selected_node_id
                    .unwrap_or_else(|| "none".to_string())
            );
        }
        Commands::Nodes { json } => {
            let nodes = sample_nodes();
            if json {
                if let Err(error) = print_json(&nodes) {
                    eprintln!("failed to print json: {error}");
                    std::process::exit(1);
                }
                return;
            }

            print_nodes_table();
        }
        Commands::Contribute { m, cuda, percent } => {
            if !config_exists() {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            if let Ok((identity, _, _)) = load_or_create_identity() {
                config.device_id = device_id_for_identity(&identity);
                config.public_key_fingerprint = Some(identity.fingerprint);
            }
            config.backend_preference = match (m, cuda) {
                (true, false) => Backend::M,
                (false, true) => Backend::Cuda,
                (false, false) => Backend::Auto,
                (true, true) => {
                    eprintln!("choose only one backend: --m or --cuda");
                    std::process::exit(1);
                }
            };

            if let Some(percent) = percent {
                config.contribution_percent = percent;
            }

            match save_config(&config) {
                Ok(path) => println!(
                    "set backend preference to {} at {}% (config: {})",
                    config.backend_preference,
                    config.contribution_percent,
                    path.display()
                ),
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Pause => {
            if !config_exists() {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            config.paused = true;

            if let Err(error) = save_config(&config) {
                eprintln!("failed to save config: {error}");
                std::process::exit(1);
            }
            println!("paused contribution");
        }
        Commands::Resume => {
            if !config_exists() {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            config.paused = false;

            if let Err(error) = save_config(&config) {
                eprintln!("failed to save config: {error}");
                std::process::exit(1);
            }
            println!("resumed contribution");
        }
        Commands::Config { command } => match command {
            ConfigCommands::Path => {
                println!("{}", resolved_config_path().display());
            }
            ConfigCommands::Show { json } => {
                let config = current_config_or_default();
                if json {
                    if let Err(error) = print_json(&config) {
                        eprintln!("failed to print json: {error}");
                        std::process::exit(1);
                    }
                } else {
                    print_config_summary(&config, &resolved_config_path());
                }
            }
            ConfigCommands::Set { key, value } => {
                if !config_exists() {
                    eprintln!("run \"opengpu init\" first");
                    std::process::exit(1);
                }

                let mut config = current_config_or_default();
                let result = match key {
                    ConfigKey::ControlPlaneUrl => {
                        config.control_plane_url = value;
                        Ok("control plane URL updated".to_string())
                    }
                    ConfigKey::ProfileName => {
                        config.profile_name = if value.trim().is_empty() {
                            None
                        } else {
                            Some(value)
                        };
                        Ok("profile name updated".to_string())
                    }
                    ConfigKey::Backend => match value.parse::<Backend>() {
                        Ok(backend) => {
                            config.backend_preference = backend;
                            Ok(format!("backend preference updated to {backend}"))
                        }
                        Err(error) => Err(error),
                    },
                    ConfigKey::DeviceId => {
                        if value.trim().is_empty() {
                            Err("device id cannot be empty".to_string())
                        } else {
                            config.device_id = value;
                            Ok("device id updated".to_string())
                        }
                    }
                    ConfigKey::ContributionPercent => match value.parse::<u8>() {
                        Ok(percent) if (1..=100).contains(&percent) => {
                            config.contribution_percent = percent;
                            Ok(format!("contribution percent updated to {percent}%"))
                        }
                        Ok(_) => Err("contribution percent must be between 1 and 100".to_string()),
                        Err(_) => Err("contribution percent must be a number".to_string()),
                    },
                };

                match result {
                    Ok(message) => match save_config(&config) {
                        Ok(path) => {
                            println!("{message}");
                            println!("config saved at {}", path.display());
                        }
                        Err(error) => {
                            eprintln!("failed to save config: {error}");
                            std::process::exit(1);
                        }
                    },
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                }
            }
            ConfigCommands::Reset { yes } => {
                if !yes {
                    eprintln!("refusing to reset without --yes");
                    std::process::exit(1);
                }

                match remove_config_files() {
                    Ok(()) => println!("reset local config"),
                    Err(error) => {
                        eprintln!("failed to reset config: {error}");
                        std::process::exit(1);
                    }
                }
            }
        },
        Commands::Doctor => {
            if let Err(error) = print_doctor() {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        Commands::Logs => {
            let config = current_config_or_default();
            println!("logSource: local cli state");
            println!("configPath: {}", resolved_config_path().display());
            println!("connected: {}", if config.connected { "yes" } else { "no" });
            println!("paused: {}", if config.paused { "yes" } else { "no" });
            println!(
                "note: worker and control-plane logs will appear once those services are online"
            );
        }
        Commands::Update => {
            println!("updateChannel: GitHub Releases");
            println!("tagPattern: cli-v*");
            println!("installer: https://novusx.ai/install");
        }
    }
}
