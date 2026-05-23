mod config;
mod identity;
mod model;
mod model_catalog;
mod routing;
mod types;

use clap::{Parser, Subcommand};
use crossterm::event::{read, Event, KeyCode, KeyModifiers};
use crossterm::style::{style, Color, Stylize};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use serde::Serialize;
use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::process::Command;
use std::thread;
use types::Backend;

use config::{config_exists, load_config, resolved_config_path, save_config, Config};
use identity::{
    device_id_for_identity, ensure_identity, load_identity, load_or_create_identity,
};
use model::{
    active_model_name, add_model, configured_model_dir_string, ensure_effective_model_dir,
    list_models, prune_models, remove_model, use_model, ModelRecord,
};
use model_catalog::{selection_for, ModelOption};

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
    /// Start the OpenGPU network
    Start,
    /// Join the OpenGPU network (runs init on first use)
    Connect,
    /// Store local operator auth state
    Login {
        #[arg(long)]
        token: Option<String>,
    },
    /// Clear local operator auth state
    Logout,
    /// Leave the OpenGPU network
    Disconnect,
    /// Show current node status
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Manage the local model cache
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },
    /// Update the OpenGPU binary
    Update,
}

#[derive(Subcommand, Debug)]
enum ModelCommands {
    /// List cached models
    List {
        #[arg(long)]
        json: bool,
    },
    /// Download or cache a model and mark it active
    Use {
        name: String,
    },
    /// Download or cache a model without switching to it
    Add {
        name: String,
    },
    /// Remove a cached model
    Remove {
        name: String,
        #[arg(long)]
        force: bool,
    },
    /// Remove inactive cached models
    Prune {
        #[arg(long)]
        yes: bool,
    },
}

fn current_config_or_default() -> Config {
    load_config().ok().flatten().unwrap_or_default()
}

fn resolved_backend(config: &Config) -> Backend {
    if config.backend_preference.is_auto() {
        detect_backend()
    } else {
        config.backend_preference
    }
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

fn display_public_key_hex(config: &Config) -> String {
    if let Some(public_key) = config.public_key_fingerprint.as_ref() {
        if let Ok(Some(identity)) = load_identity() {
            if identity.fingerprint == *public_key {
                return identity.public_key_hex;
            }
        }
    }

    load_identity()
        .ok()
        .flatten()
        .map(|identity| identity.public_key_hex)
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

fn colored_state(value: bool, active_color: Color, active_text: &str, inactive_text: &str) -> String {
    if value {
        style(active_text).with(active_color).to_string()
    } else {
        style(inactive_text).with(Color::DarkGrey).to_string()
    }
}

#[derive(Clone, Debug)]
struct PowerState {
    source: String,
    on_battery: bool,
    battery_percent: Option<u8>,
}

fn probe_power_state() -> PowerState {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("pmset").args(["-g", "batt"]).output() {
            if let Ok(stdout) = String::from_utf8(output.stdout) {
                let mut source = "unknown".to_string();
                let mut on_battery = true;
                let mut battery_percent = None;

                for line in stdout.lines() {
                    if line.starts_with("Now drawing from") {
                        source = line
                            .split_once('\'')
                            .map(|(_, rest)| rest.trim_matches('\'').to_string())
                            .unwrap_or_else(|| line.to_string());
                        on_battery = !source.to_lowercase().contains("ac power");
                    }

                    if let Some(percent_text) = line.split('%').next() {
                        if let Some(token) = percent_text
                            .split_whitespace()
                            .rev()
                            .find(|part| part.chars().all(|ch| ch.is_ascii_digit()))
                        {
                            battery_percent = token.parse::<u8>().ok();
                        }
                    }
                }

                return PowerState {
                    source,
                    on_battery,
                    battery_percent,
                };
            }
        }
    }

    PowerState {
        source: "unknown".to_string(),
        on_battery: false,
        battery_percent: None,
    }
}

fn policy_reason(config: &Config, power: &PowerState, active_model: Option<&str>) -> Option<String> {
    if config.contribution_percent == 0 {
        return Some("contribution percent is unset".to_string());
    }

    if active_model.is_none() {
        return Some("no active model is selected".to_string());
    }

    if power.on_battery {
        if config.contribution_percent > 20 {
            return Some("battery power requires contribution percent <= 20".to_string());
        }

        if let Some(percent) = power.battery_percent {
            if percent <= 20 {
                return Some("battery level is too low to start work safely".to_string());
            }
        }
    }

    None
}

fn policy_allowed(config: &Config, power: &PowerState, active_model: Option<&str>) -> bool {
    policy_reason(config, power, active_model).is_none()
}

fn provider_count(config: &Config, power: &PowerState, active_model: Option<&str>) -> usize {
    if config.connected && !config.paused && policy_allowed(config, power, active_model) {
        1
    } else {
        0
    }
}

fn print_config_summary(config: &Config, path: &std::path::Path) {
    let detected_backend = resolved_backend(config);
    let power = probe_power_state();
    let active_model = active_model_name(config);
    let allowed = policy_allowed(config, &power, active_model.as_deref());
    let provider_count = provider_count(config, &power, active_model.as_deref());
    println!("configPath: {}", path.display());
    println!("deviceId: {}", config.device_id);
    println!("publicKey: {}", display_public_key_hex(config));
    println!(
        "publicKeyFingerprint: {}",
        display_public_key_fingerprint(config)
    );
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
    println!(
        "connected: {}",
        colored_state(config.connected, Color::Green, "yes", "no")
    );
    println!(
        "paused: {}",
        colored_state(
            config.paused,
            Color::AnsiValue(208),
            "yes",
            "no"
        )
    );
    println!("backendPreference: {}", config.backend_preference);
    println!("detectedBackend: {}", detected_backend);
    println!("providerCount: {}", provider_count);
    println!(
        "modelDir: {}",
        configured_model_dir_string(config)
    );
    println!(
        "activeModel: {}",
        active_model.clone().unwrap_or_else(|| "unset".to_string())
    );
    println!(
        "contributionPercent: {}",
        if config.contribution_percent == 0 {
            "unset".to_string()
        } else {
            format!("{}%", config.contribution_percent)
        }
    );
    println!("controlPlaneUrl: {}", config.control_plane_url);
    println!("powerSource: {}", power.source);
    println!("onBattery: {}", if power.on_battery { "yes" } else { "no" });
    println!(
        "batteryPercent: {}",
        power
            .battery_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!("policyAllowed: {}", if allowed { "yes" } else { "no" });
    if let Some(reason) = policy_reason(config, &power, active_model.as_deref()) {
        println!("policyReason: {}", reason);
    }
}

fn print_startup_summary(config: &Config, path: &std::path::Path) {
    let detected_backend = resolved_backend(config);
    let cores = thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let power = probe_power_state();
    let active_model = active_model_name(config);
    let allowed = policy_allowed(config, &power, active_model.as_deref());

    println!("startup ready for {}", config.device_id);
    println!("publicKey: {}", display_public_key_hex(config));
    println!(
        "publicKeyFingerprint: {}",
        display_public_key_fingerprint(config)
    );
    println!("platform: {}-{}", env::consts::OS, env::consts::ARCH);
    println!("cpuCores: {}", cores);
    println!("backendPreference: {}", config.backend_preference);
    println!("detectedBackend: {}", detected_backend);
    println!(
        "modelDir: {}",
        configured_model_dir_string(config)
    );
    println!(
        "activeModel: {}",
        active_model.clone().unwrap_or_else(|| "unset".to_string())
    );
    println!(
        "contributionPercent: {}",
        if config.contribution_percent == 0 {
            "unset".to_string()
        } else {
            format!("{}%", config.contribution_percent)
        }
    );
    println!("connected: {}", colored_state(true, Color::Green, "yes", "no"));
    println!(
        "paused: {}",
        colored_state(false, Color::AnsiValue(208), "yes", "no")
    );
    println!("configPath: {}", path.display());
    println!("powerSource: {}", power.source);
    println!("onBattery: {}", if power.on_battery { "yes" } else { "no" });
    println!(
        "batteryPercent: {}",
        power
            .battery_percent
            .map(|value| format!("{value}%"))
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!("policyAllowed: {}", if allowed { "yes" } else { "no" });
    if let Some(reason) = policy_reason(config, &power, active_model.as_deref()) {
        println!("policyReason: {}", reason);
    }
}

fn print_model_inventory(config: &Config, models: &[ModelRecord], json: bool) {
    if json {
        let payload = serde_json::json!({
            "model_dir": configured_model_dir_string(config),
            "active_model": active_model_name(config),
            "models": models,
        });
        if let Err(error) = print_json(&payload) {
            eprintln!("failed to print json: {error}");
            std::process::exit(1);
        }
        return;
    }

    let active_model = active_model_name(config).unwrap_or_else(|| "unset".to_string());
    let subtitle = format!("active model: {}", active_model);
    let mut body = vec![
        format!("model dir: {}", configured_model_dir_string(config)),
        format!("cache size: {} model(s)", models.len()),
    ];

    if models.is_empty() {
        body.push("models: none cached".to_string());
    } else {
        body.push("cached models:".to_string());
        for model in models {
            let state = if model.active { "ACTIVE" } else { "cached" };
            let prefix = if model.active { ">>" } else { "  " };
            body.push(format!("{prefix} {:<28} [{state}]", model.name));
        }
    }

    print_retro_panel("MODEL CACHE", &subtitle, &body, Color::Cyan);
}

fn print_model_event(title: &str, model_name: &str, detail: &str, accent: Color, config: &Config) {
    let body = vec![
        format!("model: {}", model_name),
        format!("detail: {}", detail),
        format!("model dir: {}", configured_model_dir_string(config)),
        format!(
            "active model: {}",
            active_model_name(config).unwrap_or_else(|| "unset".to_string())
        ),
    ];
    print_retro_panel(title, "local cache updated", &body, accent);
}

fn read_operator_token_from_prompt() -> Result<String, String> {
    if !io::stdin().is_terminal() {
        return Err("missing token; pass --token or use an interactive terminal".to_string());
    }

    print!("Operator token: ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut token = String::new();
    io::stdin()
        .read_line(&mut token)
        .map_err(|error| error.to_string())?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("operator token cannot be empty".to_string());
    }
    Ok(token)
}

fn print_retro_panel(title: &str, subtitle: &str, lines: &[String], accent: Color) {
    let mut width = title.chars().count().max(subtitle.chars().count());
    for line in lines {
        width = width.max(line.chars().count());
    }
    let inner_width = width + 2;
    let top = format!("╭{}╮", "─".repeat(inner_width));
    let bottom = format!("╰{}╯", "─".repeat(inner_width));
    println!("{}", style(top).with(Color::DarkGrey));
    println!(
        "{}",
        style(format!("│ {:<width$} │", title.to_uppercase(), width = width))
            .with(accent)
            .bold()
    );
    println!(
        "{}",
        style(format!("│ {:<width$} │", subtitle, width = width)).with(Color::DarkGrey)
    );
    println!("{}", style(format!("├{}┤", "─".repeat(inner_width))).with(Color::DarkGrey));
    for line in lines {
        println!("│ {:<width$} │", line, width = width);
    }
    println!("{}", style(bottom).with(Color::DarkGrey));
}

fn contribution_semantics(backend: Backend) -> &'static str {
    match backend {
        Backend::M => "memory-and-compute budget for Apple Silicon M-series",
        Backend::Cuda => "automatic routing budget",
        Backend::Auto => "automatic routing budget",
    }
}

fn detect_backend() -> Backend {
    if env::consts::OS == "macos" && env::consts::ARCH == "aarch64" {
        return Backend::M;
    }

    Backend::Auto
}

enum PromptOutcome {
    Selected(u8),
    Cancelled,
}

fn prompt_contribution_percent(default_percent: u8) -> PromptOutcome {
    const OPTIONS: &[(u8, &str)] = &[
        (20, "light"),
        (30, "balanced"),
        (50, "strong"),
        (75, "aggressive"),
        (90, "max"),
    ];

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        let mut input = String::new();
        if io::stdin().read_to_string(&mut input).is_ok() {
            let choice = input.trim();
            const OPTIONS: [u8; 5] = [20, 30, 50, 75, 90];
            if let Ok(value) = choice.parse::<usize>() {
                if (1..=OPTIONS.len()).contains(&value) {
                    return PromptOutcome::Selected(OPTIONS[value - 1]);
                }
            }
        }
        return PromptOutcome::Selected(default_percent);
    }

    let mut selected = OPTIONS
        .iter()
        .position(|(percent, _)| *percent == default_percent)
        .unwrap_or(1);

    if enable_raw_mode().is_err() {
        return PromptOutcome::Selected(default_percent);
    }

    let render_menu = |selected: usize| {
        print!("\x1b[2J\x1b[H");
        println!("Contribution level");
        println!("-------------------");
        for (index, (percent, label)) in OPTIONS.iter().enumerate() {
            let marker = if index == selected { ">>" } else { "  " };
            println!("{marker} {percent:>2}% - {label}");
        }
        println!();
        println!("Use ↑/↓ and Enter");
        let _ = io::stdout().flush();
    };

    render_menu(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    return PromptOutcome::Cancelled;
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    render_menu(selected);
                }
                KeyCode::Down => {
                    if selected + 1 < OPTIONS.len() {
                        selected += 1;
                    }
                    render_menu(selected);
                }
                KeyCode::Enter => break Some(OPTIONS[selected].0),
                KeyCode::Esc => break None,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };

    let _ = disable_raw_mode();
    match result {
        Some(value) => PromptOutcome::Selected(value),
        None => PromptOutcome::Selected(default_percent),
    }
}

fn detect_memory_gb() -> u64 {
    #[cfg(target_os = "macos")]
    {
        // sysctl hw.memsize returns total unified memory in bytes
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output();
        if let Ok(out) = output {
            if let Ok(s) = std::str::from_utf8(&out.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return bytes / (1024 * 1024 * 1024);
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        // /proc/meminfo MemTotal in kB
        if let Ok(contents) = std::fs::read_to_string("/proc/meminfo") {
            for line in contents.lines() {
                if line.starts_with("MemTotal:") {
                    let kb: u64 = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    return kb / (1024 * 1024);
                }
            }
        }
    }
    8 // safe fallback
}

enum ModelChoice {
    Model(ModelOption),
    LocalPath(String),
}

fn prompt_model_selection(backend: Backend) -> ModelChoice {
    let gb = detect_memory_gb();
    let selection = selection_for(backend, gb);

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return ModelChoice::Model(selection.recommended);
    }

    const LOCAL_OPT: usize = 2;
    let options = [selection.lighter.clone(), selection.recommended.clone()];
    let mut selected: usize = 1; // start on recommended

    if enable_raw_mode().is_err() {
        return ModelChoice::Model(selection.recommended);
    }

    let render = |selected: usize| {
        print!("\x1b[2J\x1b[H");
        println!("Which model should this node run?");
        println!("detected: {} / {}GB memory", selection.backend, selection.memory_gb);
        println!("----------------------------------");
        for (i, option) in options.iter().enumerate() {
            let marker = if i == selected { ">>" } else { "  " };
            println!(
                "{marker} {}. {} [{}] — {}",
                i + 1,
                option.label,
                option.name,
                option.notes
            );
        }
        println!();
        println!("Use ↑/↓ and Enter — you must choose one");
        let _ = io::stdout().flush();
    };

    render(selected);

    let result = loop {
        match read() {
            Ok(Event::Key(key)) => match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    println!();
                    eprintln!("cancelled");
                    std::process::exit(130);
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    render(selected);
                }
                KeyCode::Down => {
                    if selected + 1 < options.len() {
                        selected += 1;
                    }
                    render(selected);
                }
                KeyCode::Enter => break selected,
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break 1,
        }
    };

    let _ = disable_raw_mode();

    if result == LOCAL_OPT {
        print!("\x1b[2J\x1b[H");
        print!("Path to your models directory: ");
        let _ = io::stdout().flush();
        let mut path = String::new();
        let _ = io::stdin().read_line(&mut path);
        let path = path.trim().to_string();
        if path.is_empty() {
            // still can't skip — fall back to recommended
            ModelChoice::Model(selection.recommended)
        } else {
            ModelChoice::LocalPath(path)
        }
    } else {
        ModelChoice::Model(options[result].clone())
    }
}

fn run_init() -> Config {
    let (identity, created, identity_path) = match ensure_identity() {
        Ok(result) => result,
        Err(error) => {
            eprintln!("failed to initialize identity: {error}");
            std::process::exit(1);
        }
    };

    let mut config = config_from_identity(&identity);
    config.backend_preference = detect_backend();

    // model selection — mandatory, no skip
    match prompt_model_selection(config.backend_preference) {
        ModelChoice::Model(model) => {
            ensure_effective_model_dir(&mut config);
            if let Err(error) = use_model(&mut config, &model.name) {
                eprintln!("failed to cache model `{}`: {error}", model.name);
                std::process::exit(1);
            }
            print_model_event(
                "MODEL SELECTED",
                &model.name,
                "starter model recorded in local cache",
                Color::Cyan,
                &config,
            );
        }
        ModelChoice::LocalPath(path) => {
            config.model_dir = Some(path);
            config.active_model = None;
            config.models = vec![];
        }
    }

    if config.contribution_percent == 0 {
        match prompt_contribution_percent(30) {
            PromptOutcome::Selected(percent) => {
                config.contribution_percent = percent;
            }
            PromptOutcome::Cancelled => {
                eprintln!("cancelled");
                std::process::exit(130);
            }
        }
    }

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

    config
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start | Commands::Connect => {
            // auto-init on first run
            if !config_exists() {
                run_init();
            }

            let mut config = current_config_or_default();
            if let Ok((identity, _, _)) = load_or_create_identity() {
                config.device_id = device_id_for_identity(&identity);
                config.public_key_fingerprint = Some(identity.fingerprint);
            }
            if let Some(active_model) = active_model_name(&config) {
                if let Err(error) = use_model(&mut config, &active_model) {
                    eprintln!("failed to refresh active model `{active_model}`: {error}");
                    std::process::exit(1);
                }
            }
            config.connected = true;
            config.paused = false;

            match save_config(&config) {
                Ok(_) => {
                    print_startup_summary(&config, &resolved_config_path());
                    println!(
                        "contributionMeaning: {}",
                        contribution_semantics(config.backend_preference)
                    );
                }
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Login { token } => {
            let mut config = current_config_or_default();
            let token = match token {
                Some(token) => token.trim().to_string(),
                None => read_operator_token_from_prompt().unwrap_or_else(|error| {
                    eprintln!("{error}");
                    std::process::exit(1);
                }),
            };

            if token.is_empty() {
                eprintln!("operator token cannot be empty");
                std::process::exit(1);
            }

            config.auth_token = Some(token);
            match save_config(&config) {
                Ok(path) => {
                    println!("authenticated: yes");
                    println!("authTokenPath: {}", path.display());
                }
                Err(error) => {
                    eprintln!("failed to save auth token: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Logout => {
            let mut config = current_config_or_default();
            config.auth_token = None;
            match save_config(&config) {
                Ok(_) => {
                    println!("authenticated: no");
                }
                Err(error) => {
                    eprintln!("failed to clear auth token: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Disconnect => {
            if !config_exists() {
                eprintln!("not connected");
                std::process::exit(1);
            }

            let mut config = current_config_or_default();
            config.connected = false;
            config.paused = false;

            match save_config(&config) {
                Ok(_) => {
                    println!("disconnected {}", config.device_id);
                    println!("connected: no");
                }
                Err(error) => {
                    eprintln!("failed to save config: {error}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Status { json } => {
            let config = current_config_or_default();
            let preferred_backend = resolved_backend(&config);
            let power = probe_power_state();
            let active_model = active_model_name(&config);
            let policy_allowed = policy_allowed(&config, &power, active_model.as_deref());
            let provider_count = if config.connected && !config.paused && policy_allowed {
                1
            } else {
                0
            };

            if json {
                let payload = serde_json::json!({
                    "config": config,
                    "detected_backend": preferred_backend,
                    "provider_count": provider_count,
                    "routing_mode": "local-only",
                    "selected_provider": if provider_count == 1 { "self" } else { "none" },
                    "power_state": {
                        "source": power.source,
                        "on_battery": power.on_battery,
                        "battery_percent": power.battery_percent,
                    },
                    "policy_allowed": policy_allowed,
                    "policy_reason": policy_reason(&config, &power, active_model.as_deref()),
                });
                if let Err(error) = print_json(&payload) {
                    eprintln!("failed to print json: {error}");
                    std::process::exit(1);
                }
                return;
            }

            print_config_summary(&config, &resolved_config_path());
            println!("routingMode: local-only");
            println!(
                "selectedProvider: {}",
                if provider_count == 1 { "self" } else { "none" }
            );
        }
        Commands::Model { command } => {
            let mut config = current_config_or_default();
            match command {
                ModelCommands::List { json } => {
                    let models = match list_models(&config) {
                        Ok(models) => models,
                        Err(error) => {
                            eprintln!("failed to read model cache: {error}");
                            std::process::exit(1);
                        }
                    };
                    print_model_inventory(&config, &models, json);
                }
                ModelCommands::Use { name } => {
                    if let Err(error) = use_model(&mut config, &name) {
                        eprintln!("failed to activate model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    print_model_event(
                        "MODEL SWITCHED",
                        &name,
                        "activated and ready for worker launch",
                        Color::Green,
                        &config,
                    );
                }
                ModelCommands::Add { name } => {
                    if let Err(error) = add_model(&mut config, &name) {
                        eprintln!("failed to cache model `{name}`: {error}");
                        std::process::exit(1);
                    }
                    if let Err(error) = save_config(&config) {
                        eprintln!("failed to save config: {error}");
                        std::process::exit(1);
                    }
                    print_model_event(
                        "MODEL CACHED",
                        &name,
                        "added to local cache without switching",
                        Color::Cyan,
                        &config,
                    );
                }
                ModelCommands::Remove { name, force } => {
                    match remove_model(&mut config, &name, force) {
                        Ok(true) => {
                            if let Err(error) = save_config(&config) {
                                eprintln!("failed to save config: {error}");
                                std::process::exit(1);
                            }
                            print_model_event(
                                "MODEL REMOVED",
                                &name,
                                "cache entry deleted",
                                Color::DarkYellow,
                                &config,
                            );
                        }
                        Ok(false) => {
                            eprintln!("model not found: {name}");
                            std::process::exit(1);
                        }
                        Err(error) => {
                            eprintln!("{error}");
                            std::process::exit(1);
                        }
                    }
                }
                ModelCommands::Prune { yes } => {
                    if !yes {
                        eprintln!("refusing to prune without --yes");
                        std::process::exit(1);
                    }
                    match prune_models(&mut config) {
                        Ok(removed) => {
                            if let Err(error) = save_config(&config) {
                                eprintln!("failed to save config: {error}");
                                std::process::exit(1);
                            }
                            print_model_event(
                                "MODEL PRUNED",
                                &format!("{removed} removed"),
                                "inactive cache entries cleared",
                                Color::DarkYellow,
                                &config,
                            );
                        }
                        Err(error) => {
                            eprintln!("failed to prune models: {error}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Commands::Update => {
            println!("updateChannel: GitHub Releases");
            println!("tagPattern: cli-v*");
            println!("installer: https://novusx.ai/install");
        }
    }
}
