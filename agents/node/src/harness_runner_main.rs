#[path = "harness_runner_config.rs"]
mod config;
mod harness;
mod harness_client;
mod harness_executor;
mod harness_loop;
mod harness_tools;
mod http;
#[path = "../../../apps/cli/src/identity.rs"]
mod identity;

use clap::{Parser, Subcommand};
use config::{config_path, load_config, save_config, RunnerConfig};
use identity::{load_or_create_identity, DeviceIdentity};
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "mundusx-harness-runner",
    version,
    about = "User-owned MundusX repository and validation runner",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a separate runner configuration and secure signing identity.
    Init,
    /// Run the signed Harness claim and execution loop.
    Run {
        #[arg(long)]
        once: bool,
        #[arg(long, default_value_t = 5)]
        interval_seconds: u64,
    },
    /// Show bounded runner configuration without repository contents or secrets.
    Status {
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => init(),
        Command::Run {
            once,
            interval_seconds,
        } => run(once, interval_seconds),
        Command::Status { json } => status(json),
    }
}

fn init() {
    let path = config_path();
    if path.exists() {
        eprintln!("runner config already exists: {}", path.display());
        std::process::exit(1);
    }
    let config = RunnerConfig::default();
    let path = save_config(&config).unwrap_or_else(fatal_io("create runner config"));
    let (_, _, identity_path) =
        load_or_create_identity().unwrap_or_else(fatal_io("create runner identity"));
    println!("runnerConfig: {}", path.display());
    println!("runnerIdentity: {}", identity_path.display());
    println!("next: set owner_user_id, tenant_ids, repositories, workspace_root, git_executable, and validation_profiles");
}

fn run(once: bool, interval_seconds: u64) {
    let config = required_config();
    let (identity, _, _) =
        load_or_create_identity().unwrap_or_else(fatal_io("load runner identity"));
    loop {
        if let Err(error) = run_cycle(&config, &identity) {
            eprintln!("harnessRunner: {error}");
        }
        if once {
            return;
        }
        thread::sleep(Duration::from_secs(interval_seconds.max(5)));
    }
}

fn run_cycle(config: &RunnerConfig, identity: &DeviceIdentity) -> Result<(), String> {
    let capabilities = harness_client::capabilities(config)?;
    let trusted_identity = identity::trust_path() != "local-encrypted-fallback";
    harness_client::register_runner(
        config,
        identity,
        &capabilities,
        config.usable_memory_mb.max(1),
        trusted_identity,
    )?;
    println!(
        "harnessRunner: ready {} ({} slots)",
        config.runner_id,
        config.parallel_slots.clamp(1, 64)
    );

    let mut handles = Vec::new();
    for _ in 0..config.parallel_slots.clamp(1, 64) {
        let Some((task, attempt)) = harness_client::claim_next(config, identity)? else {
            break;
        };
        println!("harnessPoll: claimed {}", attempt.attempt_id);
        let config = config.clone();
        let identity = identity.clone();
        handles.push(thread::spawn(move || {
            let attempt_id = attempt.attempt_id.clone();
            match harness_client::execute_claim(&config, &identity, task, attempt) {
                Ok(()) => println!("harnessPoll: completed {attempt_id}"),
                Err(error) => eprintln!("harnessPoll: {attempt_id}: {error}"),
            }
        }));
    }
    for handle in handles {
        let _ = handle.join();
    }
    Ok(())
}

fn status(json: bool) {
    let config = required_config();
    let identity = load_or_create_identity()
        .unwrap_or_else(fatal_io("load runner identity"))
        .0;
    let capabilities = harness_client::capabilities(&config);
    let payload = serde_json::json!({
        "config_path": config_path(),
        "runner_id": config.runner_id,
        "device_id": config.device_id,
        "owner_configured": !config.owner_user_id.trim().is_empty(),
        "tenant_count": config.tenant_ids.len(),
        "repository_source_ids": config.repositories.keys().collect::<Vec<_>>(),
        "parallel_slots": config.parallel_slots.clamp(1, 64),
        "control_plane_url": config.control_plane_url,
        "inference_model": config.inference_model,
        "identity_fingerprint": identity.fingerprint,
        "identity_trust_path": identity::trust_path(),
        "capabilities": capabilities,
    });
    if json {
        println!("{}", serde_json::to_string(&payload).expect("status json"));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).expect("status json")
        );
    }
}

fn required_config() -> RunnerConfig {
    match load_config() {
        Ok(Some(config)) => config,
        Ok(None) => {
            eprintln!("missing runner config; run `mundusx-harness-runner init`");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("invalid runner config: {error}");
            std::process::exit(1);
        }
    }
}

fn fatal_io<T>(action: &'static str) -> impl FnOnce(std::io::Error) -> T {
    move |error| {
        eprintln!("failed to {action}: {error}");
        std::process::exit(1)
    }
}
