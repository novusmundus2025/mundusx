use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

mod routing;

#[derive(Parser, Debug)]
#[command(name = "opengpu", version, about = "OpenGPU CLI", arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Init,
    Login,
    Connect,
    Status,
    Contribute {
        #[arg(long)]
        m: bool,
        #[arg(long)]
        cuda: bool,
    },
    Pause,
    Resume,
    Logs,
    Update,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Config {
    version: u32,
    device_id: String,
    backend_preference: Option<String>,
    connected: bool,
    paused: bool,
    control_plane_url: String,
}

fn config_dir() -> PathBuf {
    std::env::var_os("OPENGPU_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|dir| dir.join(".opengpu")))
        .unwrap_or_else(|| PathBuf::from(".opengpu"))
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn local_config_path() -> PathBuf {
    PathBuf::from(".opengpu").join("config.json")
}

fn resolve_config_path() -> PathBuf {
    let home = config_path();
    if home.exists() {
        return home;
    }

    let local = local_config_path();
    if local.exists() {
        return local;
    }

    home
}

fn default_config() -> Config {
    Config {
        version: 1,
        device_id: format!("node-{}", uuid::Uuid::new_v4().simple()),
        backend_preference: None,
        connected: false,
        paused: false,
        control_plane_url: "https://api.opengpu.ai".to_string(),
    }
}

fn read_config() -> Option<Config> {
    let path = resolve_config_path();
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_config(config: &Config) -> std::io::Result<PathBuf> {
    let path = config_path();
    let data = serde_json::to_string_pretty(config).expect("config serialization");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    match fs::write(&path, format!("{data}\n")) {
        Ok(()) => Ok(path),
        Err(_) => {
            let fallback = local_config_path();
            if let Some(parent) = fallback.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&fallback, format!("{data}\n"))?;
            Ok(fallback)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NodeStatus {
    node_id: String,
    backend: String,
    status: String,
    available_memory_mb: u32,
    available_gpu_percent: u32,
    label: String,
}

fn sample_nodes() -> Vec<NodeStatus> {
    vec![
        NodeStatus {
            node_id: "m-001".to_string(),
            backend: "m".to_string(),
            status: "idle".to_string(),
            available_memory_mb: 24_576,
            available_gpu_percent: 72,
            label: "MacBook M-series".to_string(),
        },
        NodeStatus {
            node_id: "cuda-001".to_string(),
            backend: "cuda".to_string(),
            status: "online".to_string(),
            available_memory_mb: 49_152,
            available_gpu_percent: 84,
            label: "CUDA Worker".to_string(),
        },
        NodeStatus {
            node_id: "cuda-002".to_string(),
            backend: "cuda".to_string(),
            status: "offline".to_string(),
            available_memory_mb: 32_768,
            available_gpu_percent: 0,
            label: "Dead CUDA Worker".to_string(),
        },
    ]
}

fn choose_best_node(preference: Option<&str>) -> Option<NodeStatus> {
    let mut live_nodes: Vec<NodeStatus> = sample_nodes()
        .into_iter()
        .filter(|node| node.status != "offline")
        .collect();

    if let Some(preferred) = preference {
        let preferred_nodes: Vec<NodeStatus> = live_nodes
            .iter()
            .cloned()
            .filter(|node| node.backend == preferred)
            .collect();

        if !preferred_nodes.is_empty() {
            live_nodes = preferred_nodes;
        }
    }

    live_nodes.into_iter().max_by(|a, b| {
        let a_score = routing::score_node(a);
        let b_score = routing::score_node(b);
        a_score.partial_cmp(&b_score).unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn print_status(config: &Config) {
    let selected = choose_best_node(config.backend_preference.as_deref());

    println!("deviceId: {}", config.device_id);
    println!("connected: {}", if config.connected { "yes" } else { "no" });
    println!("paused: {}", if config.paused { "yes" } else { "no" });
    println!(
        "backendPreference: {}",
        config.backend_preference.as_deref().unwrap_or("auto")
    );
    println!("controlPlaneUrl: {}", config.control_plane_url);
    println!(
        "bestLiveNode: {}",
        selected
            .map(|node| format!("{} ({})", node.node_id, node.backend))
            .unwrap_or_else(|| "none".to_string())
    );
}

fn main() {
    let cli = Cli::parse();
    let config_path = resolve_config_path();
    let config_exists = config_path.exists();
    let mut config = if config_exists {
        read_config().unwrap_or_else(default_config)
    } else {
        default_config()
    };

    match cli.command {
        Commands::Init => {
            if config_exists {
                println!("config already exists at {}", config_path.display());
                return;
            }

            match write_config(&config) {
                Ok(path) => println!("initialized {}", path.display()),
                Err(error) => {
                    eprintln!("failed to initialize config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Login => {
            config.connected = false;
            match write_config(&config) {
                Ok(path) => println!("login flow not wired yet, but config is ready at {}", path.display()),
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Connect => {
            if !config_exists {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }
            config.connected = true;
            config.paused = false;
            if let Err(error) = write_config(&config) {
                eprintln!("failed to save config: {error}");
                std::process::exit(1);
            }
            println!("connected device {}", config.device_id);
        }
        Commands::Status => {
            if !config_exists {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }
            print_status(&config);
        }
        Commands::Contribute { m, cuda } => {
            if !config_exists {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }

            match (m, cuda) {
                (true, false) => config.backend_preference = Some("m".to_string()),
                (false, true) => config.backend_preference = Some("cuda".to_string()),
                _ => {
                    eprintln!("choose a backend with --m or --cuda");
                    std::process::exit(1);
                }
            }

            if let Err(error) = write_config(&config) {
                eprintln!("failed to save config: {error}");
                std::process::exit(1);
            }
            println!(
                "set backend preference to {}",
                config.backend_preference.as_deref().unwrap_or("auto")
            );
        }
        Commands::Pause => {
            if !config_exists {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }
            config.paused = true;
            if let Err(error) = write_config(&config) {
                eprintln!("failed to save config: {error}");
                std::process::exit(1);
            }
            println!("paused contribution");
        }
        Commands::Resume => {
            if !config_exists {
                eprintln!("run \"opengpu init\" first");
                std::process::exit(1);
            }
            config.paused = false;
            if let Err(error) = write_config(&config) {
                eprintln!("failed to save config: {error}");
                std::process::exit(1);
            }
            println!("resumed contribution");
        }
        Commands::Logs => {
            println!("no logs yet; node agent and control plane are still stubs");
        }
        Commands::Update => {
            println!("update channel not wired yet");
        }
    }
}
