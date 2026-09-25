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
    if let Some(value) = workloads {
        config.contribution.operations = parse_operations(value)?;
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        println!("Contribution workloads: llm,image,image-edit,video,image-to-video or all");
        println!("Qwen image and Wan video local verification are available. Video queue serving is available after verification; image dispatch and editing remain pending.");
        let current = config
            .contribution
            .operations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        print!("Workloads [{current}] (Enter keeps current): ");
        io::stdout().flush().map_err(|error| error.to_string())?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|error| error.to_string())?;
        if !answer.trim().is_empty() {
            config.contribution.operations = parse_operations(answer.trim())?;
        }
    }
    if let Some(value) = endpoint {
        config.contribution.comfyui_url = Some(validate_endpoint(value)?);
    }
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
    let endpoint = config
        .contribution
        .comfyui_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:8188");
    serde_json::json!({
        "contract_version": 1,
        "selected_operations": config.contribution.operations,
        "execution_support": crate::contribution_contract::ExecutionCapabilities::llm_only(config.contribution.llm_enabled()),
        "llm_readiness": "Use opengpu doctor or node health; selection alone does not establish readiness",
        "media_status": if config.contribution.media_enabled() { "local_image_and_queued_video" } else { "disabled" },
        "media_reason": "Qwen image local generation and Wan video queue serving are available; image dispatch and editing remain pending",
        "local_image_verification": crate::media_runtime::verification(&crate::config::config_dir(), config.contribution.comfyui_url.as_deref(), config.contribution_percent),
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
        println!("Media: use opengpu media --video setup/verify, then media serve for queued video work");
        if let Some(verification) = report.get("local_image_verification").filter(|value| !value.is_null()) {
            println!("Local image verification: {}", if verification["ready"] == true { "passed" } else { "needs verification" });
        }
        if let Some(verification) = report.get("local_video_verification").filter(|value| !value.is_null()) {
            println!("Local video verification: {}", if verification["ready"] == true { "passed" } else { "needs verification" });
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
    let profiles: Vec<bool> = [false, true].into_iter().filter(|video| config.contribution.operations.contains(
        if *video { &Operation::TextToVideo } else { &Operation::TextToImage })).collect();
    if profiles.is_empty() {
        if requested { return Err("Enable generation with --workloads image,video or all first".into()); }
        return Ok(());
    }
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut install = requested;
    if !requested && interactive {
        println!("Selected Qwen image / Wan video setup: [1] automatic (Linux ARM64 GX10/GB10) [2] existing ComfyUI [3] later");
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
        match answer.trim() {
            "1" => { config.contribution.comfyui_url = None; install = true; }
            "2" => {
                println!("ComfyUI endpoint [http://127.0.0.1:8188]:");
                answer.clear(); io::stdin().read_line(&mut answer).map_err(|e| e.to_string())?;
                config.contribution.comfyui_url = Some(validate_endpoint(if answer.trim().is_empty() { "http://127.0.0.1:8188" } else { answer.trim() })?);
                install = true;
            }
            "" | "3" => (),
            _ => return Err("Choose 1, 2, or 3; rerun setup to continue".into()),
        }
    }
    if !install { return Ok(()); }
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
    for video in profiles {
        crate::media_runtime::run_profile(&home, config.contribution_percent, endpoint, "setup", video, &["--yes".into()])?;
        crate::media_runtime::run_profile(&home, config.contribution_percent, endpoint, "verify", video, &[])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        configure(&mut config, Some("all"), None).unwrap();
        let value = report(&config, false);
        assert_eq!(
            value["execution_support"]["operations"],
            serde_json::json!(["llm"])
        );
        assert_eq!(value["media_status"], "local_image_and_queued_video");
    }
    #[test]
    fn selections_survive_config_roundtrip() {
        let mut config = Config::default();
        configure(
            &mut config,
            Some("image,image-edit"),
            Some("http://127.0.0.1:8188"),
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
