use crate::config::Config;
use crate::contribution_contract::Operation;
use serde::Serialize;
use serde_json::Value;
use std::io::{self, IsTerminal, Read, Write};
use std::time::Duration;

pub fn parse_operations(value: &str) -> Result<Vec<Operation>, String> {
    if value.trim() == "all" {
        return Ok(vec![
            Operation::Llm,
            Operation::TextToImage,
            Operation::ImageEdit,
            Operation::TextToVideo,
            Operation::ImageToVideo,
        ]);
    }
    let mut operations = Vec::new();
    for entry in value.split(',') {
        let operation: Operation = entry.parse()?;
        if !operations.contains(&operation) {
            operations.push(operation);
        }
    }
    Ok(operations)
}

pub fn validate_endpoint(value: &str) -> Result<String, String> {
    let endpoint = value.trim().trim_end_matches('/');
    let authority = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .ok_or("ComfyUI address must begin with http:// or https://")?;
    if authority.is_empty()
        || authority.starts_with('/')
        || authority.contains('@')
        || authority.contains('?')
        || authority.contains('#')
        || authority.chars().any(char::is_whitespace)
    {
        return Err(
            "ComfyUI address must have a host and no credentials, query, fragment, or whitespace"
                .into(),
        );
    }
    Ok(endpoint.to_string())
}

pub fn configure(
    config: &mut Config,
    workloads: Option<&str>,
    endpoint: Option<&str>,
) -> Result<(), String> {
    configure_with_budget(config, workloads, endpoint, &crate::media_runtime::memory::MediaBudget::detect(config.contribution_percent))
}

fn configure_with_budget(config: &mut Config, workloads: Option<&str>, endpoint: Option<&str>, budget: &crate::media_runtime::memory::MediaBudget) -> Result<(), String> {
    let mut selection = config.contribution.clone();
    if let Some(value) = workloads {
        selection.operations = parse_operations(value)?;
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        selection.operations = crate::workload_picker::prompt(&selection.operations, budget)?;
    }
    for operation in &selection.operations { budget.require_operation(*operation)?; }
    if let Some(value) = endpoint {
        selection.comfyui_url = Some(validate_endpoint(value)?);
    }
    config.contribution = selection;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ComfyDiscovery {
    pub endpoint: String,
    pub detected: bool,
    pub node_types: usize,
    pub reason: String,
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    let response = agent
        .get(url)
        .call()
        .map_err(|_| "endpoint unavailable or returned an HTTP error".to_string())?;
    // Bound discovery responses: custom-node metadata can be large.
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read endpoint response")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("discovery response exceeds 8 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "endpoint did not return valid JSON".into())
}

fn inspect(stats: &Value, nodes: &Value) -> Result<usize, String> {
    if !stats.get("system").is_some_and(Value::is_object)
        || !stats.get("devices").is_some_and(Value::is_array)
    {
        return Err("system_stats does not match a ComfyUI response".into());
    }
    let nodes = nodes.as_object().ok_or("object_info must be a node map")?;
    if nodes.is_empty()
        || !nodes
            .values()
            .all(|node| node.get("input").is_some_and(Value::is_object))
    {
        return Err("object_info does not contain valid node definitions".into());
    }
    Ok(nodes.len())
}

pub fn discover(value: &str) -> ComfyDiscovery {
    let endpoint = match validate_endpoint(value) {
        Ok(endpoint) => endpoint,
        Err(reason) => {
            return ComfyDiscovery {
                endpoint: "invalid endpoint".into(),
                detected: false,
                node_types: 0,
                reason,
            }
        }
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .redirects(0)
        .build();
    let result = get_json(&agent, &format!("{endpoint}/system_stats")).and_then(|stats| {
        let nodes = get_json(&agent, &format!("{endpoint}/object_info"))?;
        inspect(&stats, &nodes)
    });
    match result {
        Ok(node_types) => ComfyDiscovery { endpoint, detected: true, node_types,
            reason: "ComfyUI detected; model, workflow, and output verification still required. No media jobs enabled.".into() },
        Err(reason) => ComfyDiscovery { endpoint, detected: false, node_types: 0, reason },
    }
}

pub fn report(config: &Config, probe: bool) -> Value {
    let budget = crate::media_runtime::memory::MediaBudget::detect(config.contribution_percent);
    let memory_reason = config.contribution.operations.iter()
        .find_map(|operation| budget.require_operation(*operation).err());
    let endpoint = config
        .contribution
        .comfyui_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:8188");
    serde_json::json!({
        "contract_version": 1,
        "selected_operations": config.contribution.operations,
        "verified_media_profiles": crate::media_runtime::verified_profiles(&crate::config::config_dir(), config.contribution.comfyui_url.as_deref(), config.contribution_percent),
        "media_only_network_supported": true,
        "image_to_video_queue_supported": true,
        "execution_support": crate::contribution_contract::ExecutionCapabilities::llm_only(config.contribution.llm_enabled()),
        "llm_readiness": "Use opengpu doctor or node health; selection alone does not establish readiness",
        "media_eligibility": budget,
        "image_eligible": budget.allows(Operation::TextToImage),
        "video_eligible": budget.allows(Operation::TextToVideo),
        "image_minimum_budget_bytes": budget.minimum_for(Operation::TextToImage),
        "video_minimum_budget_bytes": budget.minimum_for(Operation::TextToVideo),
        "media_memory_reason": memory_reason,
        "media_status": if !config.contribution.media_enabled() { "disabled" } else if memory_reason.is_some() { "insufficient_contribution_memory" } else { "queued_image_and_video" },
        "media_reason": "Qwen image and Wan video queue serving are available after verification and admission; editing remains unavailable",
        "local_image_verification": crate::media_runtime::verification(&crate::config::config_dir(), config.contribution.comfyui_url.as_deref(), config.contribution_percent),
        "image_queue_supported": true,
        "image_queue_requires": "selected image workload, current verification, capped memory, and control-plane admission",
        "image_queue_command": "opengpu media serve --server https://chat.mundusx.ai",
        "video_queue_supported": true,
        "video_queue_requires": "selected video workload, current verification, and control-plane admission",
        "video_queue_command": "opengpu media serve --server https://chat.mundusx.ai",
        "local_video_verification": crate::media_runtime::video_verification(&crate::config::config_dir(), config.contribution.comfyui_url.as_deref(), config.contribution_percent),
        "comfyui": if probe { serde_json::to_value(discover(endpoint)).unwrap() } else { Value::Null },
    })
}

pub fn print_report(config: &Config, json: bool, probe: bool) {
    let report = report(config, probe);
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return;
    }
    println!(
        "Selected workloads: {}",
        config
            .contribution
            .operations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "LLM: {}",
        if config.contribution.llm_enabled() {
            "enabled; run opengpu doctor to check readiness"
        } else {
            "disabled"
        }
    );
    if config.contribution.media_enabled() {
        if let Some(reason) = report["media_memory_reason"].as_str() {
            println!("Media unavailable: {reason}");
        }
        if config.contribution.operations.contains(&Operation::TextToImage) {
            println!("Local image verification: {}", if report["local_image_verification"]["ready"] == true { "passed" } else { "needs verification (opengpu media verify)" });
            println!("Image queue serving requires current verification and control-plane admission.");
        }
        if config.contribution.operations.contains(&Operation::TextToVideo) {
            println!("Local video verification: {}", if report["local_video_verification"]["ready"] == true { "passed" } else { "needs verification (opengpu media --video verify)" });
            println!("Video queue serving requires an admitted contributor and a verified matching profile.");
        }
        if config.contribution.comfyui_url.is_none() {
            println!("Managed ComfyUI starts on demand; no always-running endpoint is required.");
        }
    }
    if let Some(comfy) = report.get("comfyui").filter(|value| !value.is_null()) {
        println!(
            "ComfyUI: {} — {}",
            comfy["endpoint"].as_str().unwrap_or_default(),
            comfy["reason"].as_str().unwrap_or_default()
        );
    }
}

pub fn install_media(config: &mut Config, requested: bool, yes: bool) -> Result<(), String> {
    let budget = crate::media_runtime::memory::MediaBudget::detect(config.contribution_percent);
    for operation in &config.contribution.operations { budget.require_operation(*operation)?; }
    let profiles: Vec<bool> = [false, true].into_iter().filter(|video| config.contribution.operations.contains(
        if *video { &Operation::TextToVideo } else { &Operation::TextToImage })).collect();
    if profiles.is_empty() {
        if requested { return Err("Enable generation with --workloads image,video or all first".into()); }
        return Ok(());
    }
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut install = requested;
    if !requested && interactive {
        let candidate = config.contribution.comfyui_url.as_deref().unwrap_or("http://127.0.0.1:8188");
        println!("Checking for ComfyUI at {candidate}...");
        let discovery = discover(candidate);
        let managed = (cfg!(all(target_os = "linux", target_arch = "aarch64"))
            && crate::detect_cuda_gpu_name().is_some_and(|name| name.contains("GB10")))
            || cfg!(all(target_os = "macos", target_arch = "aarch64"))
            || (cfg!(all(target_os = "windows", target_arch = "x86_64")) && crate::detect_cuda_gpu_name().is_some());
        let mut options = Vec::new();
        if managed {
            options.push(("Set up managed ComfyUI".to_string(), "Reuse cached models; download missing files and verify".to_string()));
        }
        let existing_index = options.len();
        options.push(("Use an existing ComfyUI endpoint".to_string(), if discovery.detected {
            format!("Detected at {}; verify models and workflow before use", discovery.endpoint)
        } else { "Enter an address; connect without taking ownership of the service".to_string() }));
        let later_index = options.len();
        options.push(("Set up later".to_string(), "Save choices; media may still need verification".to_string()));
        let selection = crate::select_menu_option(
            &["Image and video setup".to_string(), "Verification generates real media and can take several minutes.".to_string()],
            &options, "Up/Down: move | Enter: select | Esc: cancel", if discovery.detected { existing_index } else if managed { 0 } else { later_index },
        ).ok_or("Media setup cancelled; workload choices have been saved.")?;
        let mut answer = String::new();
        if managed && selection == 0 {
            config.contribution.comfyui_url = None;
            install = true;
        } else if selection == existing_index {
            let current = config.contribution.comfyui_url.as_deref().unwrap_or("http://127.0.0.1:8188");
            println!("ComfyUI endpoint [{current}]:");
            io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
            config.contribution.comfyui_url = Some(validate_endpoint(if answer.trim().is_empty() { current } else { answer.trim() })?);
            install = true;
        }
    }
    if !install { return Ok(()); }
    if let Some(endpoint) = config.contribution.comfyui_url.as_deref() {
        println!("Checking ComfyUI connection...");
        let discovery = discover(endpoint);
        if !discovery.detected { return Err(format!("ComfyUI connection failed: {}", discovery.reason)); }
    }
    let home = crate::config::config_dir();
    let endpoint = config.contribution.comfyui_url.as_deref();
    for video in &profiles {
        crate::media_runtime::run_profile(&home, config.contribution_percent, endpoint, "plan", *video, &[])?;
    }
    if !yes {
        if !interactive { return Err("Review opengpu media plan, then pass --yes to authorize media setup".into()); }
        println!("Install/connect the displayed profiles and generate a verification result for each? [y/N]");
        let mut answer = String::new(); io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") { return Ok(()); }
    }
    crate::config::save_config(config).map_err(|e| e.to_string())?;
    let count = profiles.len();
    for (index, video) in profiles.into_iter().enumerate() {
        let label = if video { "Wan video" } else { "Qwen image" };
        println!("[{}/{}] {label}: preparing runtime and models", index + 1, count);
        crate::media_runtime::run_profile(&home, config.contribution_percent, endpoint, "setup", video, &["--yes".into()])?;
        println!("[{}/{}] {label}: generating verification output (this can take several minutes)", index + 1, count);
        crate::media_runtime::run_profile(&home, config.contribution_percent, endpoint, "verify", video, &[])?;
        println!("[{}/{}] {label}: verification passed", index + 1, count);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripted_media_selections_require_capped_memory_and_are_atomic() {
        use crate::media_runtime::memory::{MediaBudget, GIB};
        let mut config = Config::default();
        let original = config.contribution.clone();
        for workloads in ["image", "image-edit", "video", "image-to-video", "all", "llm,image"] {
            for budget in [MediaBudget::new(Some(32 * GIB), 50), MediaBudget::new(None, 80)] {
                assert!(configure_with_budget(&mut config, Some(workloads), None, &budget).is_err());
                assert_eq!(config.contribution, original);
            }
            configure_with_budget(&mut config, Some(workloads), None, &MediaBudget::new(Some(128 * GIB), 80)).unwrap();
            config.contribution = original.clone();
        }
        configure_with_budget(&mut config, Some("llm"), None, &MediaBudget::new(None, 0)).unwrap();
        config.contribution.operations.push(Operation::TextToVideo);
        assert!(configure_with_budget(&mut config, None, None, &MediaBudget::new(Some(32 * GIB), 50)).is_err());
    }
    #[test]
    fn scripted_selection_checks_each_model_before_saving() {
        use crate::media_runtime::memory::{MediaBudget, GIB};
        let mut config = Config::default();
        let budget = MediaBudget::new(Some(64 * GIB),50);
        configure_with_budget(&mut config, Some("llm,image"), None, &budget).unwrap();
        let saved = config.contribution.clone();
        for value in ["video", "all", "llm,image,video"] {
            assert!(configure_with_budget(&mut config, Some(value), None, &budget).unwrap_err().contains("64 GiB"));
            assert_eq!(config.contribution, saved);
        }
        let budget = MediaBudget::new(Some(32 * GIB),75);
        assert!(configure_with_budget(&mut config, Some("image"), None, &budget).unwrap_err().contains("32 GiB"));
        assert!(configure_with_budget(&mut config, None, None, &budget).is_err());
    }
    #[test]
    fn discovery_reads_only_comfy_metadata() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, body) in [
                ("/system_stats", r#"{"system":{},"devices":[]}"#),
                ("/object_info", r#"{"KSampler":{"input":{}}}"#),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 8192);
                }
                assert!(String::from_utf8(request)
                    .unwrap()
                    .starts_with(&format!("GET {path} HTTP/1.1")));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let result = discover(&endpoint);
        server.join().unwrap();
        assert!(result.detected);
        assert_eq!(result.node_types, 1);
        assert!(result.reason.contains("No media jobs enabled"));
    }
    #[test]
    fn selections_allow_all_both_and_media_only() {
        assert_eq!(parse_operations("all").unwrap().len(), 5);
        assert_eq!(
            parse_operations("llm,image,llm").unwrap(),
            vec![Operation::Llm, Operation::TextToImage]
        );
        assert_eq!(parse_operations("image,image-edit").unwrap().len(), 2);
        assert!(parse_operations("").is_err());
        assert!(parse_operations("imag").is_err());
    }
    #[test]
    fn reject_credentials_and_unsafe_endpoint_forms() {
        for value in [
            "file:///tmp",
            "http://u:secret@host",
            "http:///",
            "http://host?key=secret",
            "http://host/#x",
        ] {
            assert!(validate_endpoint(value).is_err(), "{value}");
        }
        assert_eq!(
            validate_endpoint("http://127.0.0.1:8188/").unwrap(),
            "http://127.0.0.1:8188"
        );
    }
    #[test]
    fn detects_comfy_structure_without_claiming_generation_support() {
        let stats = serde_json::json!({"system":{}, "devices":[]});
        assert!(inspect(&stats, &serde_json::json!({})).is_err());
        assert!(inspect(&serde_json::json!({"ok":true}), &serde_json::json!({})).is_err());
        assert_eq!(
            inspect(&stats, &serde_json::json!({"KSampler":{"input":{}}})).unwrap(),
            1
        );
        let mut config = Config::default();
        configure_with_budget(&mut config, Some("all"), None, &crate::media_runtime::memory::MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80)).unwrap();
        let value = report(&config, false);
        assert_eq!(
            value["execution_support"]["operations"],
            serde_json::json!(["llm"])
        );
        assert_eq!(value["media_status"], "insufficient_contribution_memory");
    }
    #[test]
    fn selections_survive_config_roundtrip() {
        let mut config = Config::default();
        configure_with_budget(
            &mut config,
            Some("image,image-edit"),
            Some("http://127.0.0.1:8188"),
            &crate::media_runtime::memory::MediaBudget::new(Some(128 * 1024 * 1024 * 1024), 80),
        )
        .unwrap();
        let restored: Config =
            serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
        assert_eq!(restored.contribution, config.contribution);
        assert!(!restored.contribution.llm_enabled());
        let mut old = serde_json::to_value(&config).unwrap();
        old.as_object_mut().unwrap().remove("contribution");
        assert!(serde_json::from_value::<Config>(old)
            .unwrap()
            .contribution
            .llm_enabled());
    }
}
