use crate::types::Backend;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelOption {
    #[serde(default)]
    pub supported_os: Vec<String>,
    #[serde(default)]
    pub capabilities: serde_json::Value,
    pub name: String,
    pub label: String,
    pub notes: String,
    pub source_kind: String,
    pub source_url: String,
    pub sha256: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub backend_compatibility: Vec<Backend>,
    #[serde(default)]
    pub estimated_vram_mb: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelPreset {
    pub backend: Backend,
    pub min_memory_gb: u64,
    pub max_memory_gb: u64,
    pub lighter: ModelOption,
    pub recommended: ModelOption,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelCatalog {
    pub version: u32,
    pub presets: Vec<ModelPreset>,
}

#[derive(Clone, Debug)]
pub struct ModelSelection {
    pub backend: Backend,
    pub memory_gb: u64,
    pub lighter: ModelOption,
    pub recommended: ModelOption,
}

impl ModelOption {
    pub fn supports_backend(&self, backend: Backend) -> bool {
        backend == Backend::Auto
            || self.backend_compatibility.is_empty()
            || self.backend_compatibility.contains(&backend)
            || self.backend_compatibility.contains(&Backend::Auto)
    }
}

pub fn load_catalog() -> Result<ModelCatalog, serde_json::Error> {
    let catalog = load_unfiltered_catalog()?;
    #[cfg(not(test))]
    return Ok(catalog_for_platform(catalog, std::env::consts::OS));
    #[cfg(test)]
    Ok(catalog)
}

fn catalog_for_platform(mut catalog: ModelCatalog, platform: &str) -> ModelCatalog {
    catalog.presets = catalog.presets.into_iter().filter_map(|mut preset| {
        let allowed = |model: &ModelOption| {
            let runtime_fits = match platform {
                "macos" => model.source_kind == "huggingface-mlx",
                "windows" => model.source_kind == "huggingface-open",
                "linux" => matches!(model.source_kind.as_str(), "huggingface-open" | "huggingface-vllm"),
                _ => false,
            };
            runtime_fits && (model.supported_os.is_empty() || model.supported_os.iter().any(|os| os == platform))
        };
        match (allowed(&preset.lighter), allowed(&preset.recommended)) {
            (false, false) => None,
            (false, true) => { preset.lighter = preset.recommended.clone(); Some(preset) },
            (true, false) => { preset.recommended = preset.lighter.clone(); Some(preset) },
            (true, true) => Some(preset),
        }
    }).collect();
    catalog
}

fn load_unfiltered_catalog() -> Result<ModelCatalog, serde_json::Error> {
    if let Some(path) = std::env::var_os("OPENGPU_MODEL_CATALOG_PATH") {
        let raw = fs::read_to_string(PathBuf::from(path)).map_err(serde_json::Error::io)?;
        return serde_json::from_str(&raw);
    }

    #[cfg(not(test))]
    {
        let config = crate::config::load_config().map_err(serde_json::Error::io)?.unwrap_or_default();
        let base = config.control_plane_url.trim_end_matches('/');
        // Per-process cache avoids repeated HTTP calls during one model picker.
        static CATALOGS: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeMap<String, ModelCatalog>>> = std::sync::OnceLock::new();
        let mut catalogs = CATALOGS.get_or_init(Default::default).lock().expect("catalog cache");
        if let Some(catalog) = catalogs.get(base) { return Ok(catalog.clone()); }
        use sha2::{Digest, Sha256};
        let key = hex::encode(Sha256::digest(base.as_bytes()));
        let path = crate::config::config_dir().join(format!("model-catalog-{key}.json"));
        match fetch_catalog(base) {
            Ok(catalog) => {
                if let Ok(raw) = serde_json::to_vec(&catalog) {
                    let _ = fs::create_dir_all(crate::config::config_dir());
                    // Invalid/partial caches are rejected by parse_catalog on the next run.
                    let _ = fs::write(&path, raw);
                }
                catalogs.insert(base.to_string(), catalog.clone());
                return Ok(catalog);
            }
            Err(error) => {
                if let Some(catalog) = fs::read_to_string(&path).ok().and_then(|raw| parse_catalog(&raw).ok()) {
                    eprintln!("Model catalog unavailable; using cached allowed models for {base}.");
                    catalogs.insert(base.to_string(), catalog.clone());
                    return Ok(catalog);
                }
                // Never silently replace an admin allowlist with bundled choices.
                eprintln!("Model catalog unavailable for {base}: {error}. No cached allowlist exists. Check the URL and private-plane login before retrying.");
                return Ok(ModelCatalog { version: 1, presets: Vec::new() });
            }
        }
    }
    #[cfg(test)]
    serde_json::from_str(include_str!("../config/official-models.json"))
}

fn parse_catalog(raw: &str) -> Result<ModelCatalog, String> {
    let catalog: ModelCatalog = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    if catalog.version != 1 || catalog.presets.len() > 3000 { return Err("Unsupported model catalog".into()); }
    for model in catalog.presets.iter().flat_map(|p| [&p.lighter, &p.recommended]) {
        if model.name.is_empty() || !model.source_url.starts_with("https://huggingface.co/") { return Err("Invalid model catalog entry".into()); }
    }
    Ok(catalog)
}

fn fetch_catalog(base: &str) -> Result<ModelCatalog, String> {
    use std::io::Read;
    if !(base.starts_with("https://") || base.starts_with("http://localhost:") || base.starts_with("http://127.0.0.1:")) {
        return Err("Catalog requires HTTPS".into());
    }
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(5)).redirects(0).build();
    let request = agent.get(&format!("{}/v1/model-catalog", base.trim_end_matches('/')));
    let response = mundusx_control_plane_auth::apply(request)?.call().map_err(|error| error.to_string())?;
    let mut raw = String::new();
    response.into_reader().take(2_000_001).read_to_string(&mut raw).map_err(|error| error.to_string())?;
    if raw.len() > 2_000_000 { return Err("Model catalog too large".into()); }
    parse_catalog(&raw)
}

#[cfg(test)]
mod api_tests {
    use super::*;
    #[test]
    fn installation_catalog_is_filtered_by_os_and_admin_restrictions() {
        let catalog: ModelCatalog = serde_json::from_str(include_str!("../config/official-models.json")).unwrap();
        for (os, kinds) in [("windows", vec!["huggingface-open"]), ("macos", vec!["huggingface-mlx"]), ("linux", vec!["huggingface-open", "huggingface-vllm"])] {
            let filtered = catalog_for_platform(catalog.clone(), os);
            assert!(!filtered.presets.is_empty());
            assert!(filtered.presets.iter().flat_map(|p| [&p.lighter, &p.recommended]).all(|m| kinds.contains(&m.source_kind.as_str())));
        }
        let mut restricted = catalog_for_platform(catalog, "windows");
        for preset in &mut restricted.presets {
            preset.lighter.supported_os = vec!["linux".into()];
            preset.recommended.supported_os = vec!["linux".into()];
        }
        assert!(catalog_for_platform(restricted, "windows").presets.is_empty());
    }

    #[test]
    fn fetches_catalog_endpoint_and_preserves_an_empty_allowlist() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 2048];
            let count = stream.read(&mut buffer).unwrap();
            assert!(String::from_utf8_lossy(&buffer[..count]).starts_with("GET /v1/model-catalog "));
            let body = r#"{"version":1,"presets":[]}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let catalog = fetch_catalog(&format!("http://{address}")).unwrap();
        assert!(options_for_machine(&catalog, Backend::M, 64, Some(65536)).is_empty());
        server.join().unwrap();
        assert!(parse_catalog(r#"{"version":99,"presets":[]}"#).is_err());
    }
}

fn preset_for(catalog: &ModelCatalog, backend: Backend, memory_gb: u64) -> ModelPreset {
    catalog
        .presets
        .iter()
        .find(|preset| {
            preset.backend == backend
                && memory_gb >= preset.min_memory_gb
                && memory_gb <= preset.max_memory_gb
        })
        .or_else(|| {
            catalog.presets.iter().find(|preset| {
                preset.backend == Backend::Auto
                    && memory_gb >= preset.min_memory_gb
                    && memory_gb <= preset.max_memory_gb
            })
        })
        .or_else(|| catalog.presets.first())
        .cloned()
        .unwrap_or_else(|| fallback_preset())
}

pub fn selection_for(backend: Backend, memory_gb: u64) -> ModelSelection {
    let catalog = load_catalog().unwrap_or_else(|_| ModelCatalog { version: 1, presets: Vec::new() });
    let _catalog_version = catalog.version;
    let preset = preset_for(&catalog, backend, memory_gb);

    ModelSelection {
        backend: preset.backend,
        memory_gb,
        lighter: preset.lighter,
        recommended: preset.recommended,
    }
}

pub fn options_for_machine(
    catalog: &ModelCatalog,
    backend: Backend,
    memory_gb: u64,
    available_vram_mb: Option<u64>,
) -> Vec<ModelOption> {
    let mut options = Vec::new();
    for option in unique_catalog_options(catalog) {
        if !option.supports_backend(backend) { continue; }
        if backend == Backend::Vllm && option.source_kind != "huggingface-vllm" { continue; }
        let budget = available_vram_mb.or_else(|| (backend == Backend::M).then_some(memory_gb.saturating_mul(1024)));
        if matches!(backend, Backend::Cuda | Backend::Vllm | Backend::M) {
            if !matches!((option.estimated_vram_mb, budget), (Some(required), Some(available)) if required <= available) { continue; }
        }
        if !options.iter().any(|existing: &ModelOption| existing.name == option.name && existing.source_url == option.source_url) {
            options.push(option);
        }
    }

    options
}

pub fn selectable_options_for(
    backend: Backend,
    memory_gb: u64,
    available_vram_mb: Option<u64>,
) -> Vec<ModelOption> {
    let catalog = load_catalog().unwrap_or_else(|_| ModelCatalog { version: 1, presets: Vec::new() });
    options_for_machine(&catalog, backend, memory_gb, available_vram_mb)
}

fn unique_catalog_options(catalog: &ModelCatalog) -> Vec<ModelOption> {
    let mut options = Vec::new();

    for option in catalog
        .presets
        .iter()
        .flat_map(|preset| [preset.lighter.clone(), preset.recommended.clone()])
    {
        if options
            .iter()
            .any(|existing: &ModelOption| existing.name == option.name && existing.source_url == option.source_url && existing.backend_compatibility == option.backend_compatibility)
        {
            continue;
        }
        options.push(option);
    }

    options
}

pub fn selectable_catalog_options_for(
    backend: Backend,
    available_vram_mb: Option<u64>,
) -> Vec<ModelOption> {
    let catalog = load_catalog().unwrap_or_else(|_| ModelCatalog { version: 1, presets: Vec::new() });
    catalog
        .presets
        .into_iter()
        .flat_map(|preset| [preset.lighter, preset.recommended])
        .filter(|option| {
            option.supports_backend(backend)
                && (backend != Backend::Vllm || option.source_kind == "huggingface-vllm")
        })
        .fold(Vec::new(), |mut options, option| {
            if !options
                .iter()
                .any(|existing: &ModelOption| existing.name == option.name && existing.source_url == option.source_url)
            {
                options.push(option);
            }
            options
        })
        .into_iter()
        .filter(|option| {
            if !matches!(backend, Backend::Cuda | Backend::M | Backend::Vllm) {
                return true;
            }

            match (option.estimated_vram_mb, available_vram_mb) {
                (Some(estimated), Some(available)) => estimated <= available,
                _ => false,
            }
        })
        .collect()
}

pub fn lookup_model_for_backend(name: &str, backend: Backend) -> Option<ModelOption> {
    let catalog = load_catalog().ok()?;
    catalog
        .presets
        .into_iter()
        .flat_map(|preset| [preset.lighter, preset.recommended])
        .find(|option| option.name == name && option.supports_backend(backend))
}

pub fn lookup_model(name: &str) -> Option<ModelOption> {
    let catalog = load_catalog().ok()?;
    unique_catalog_options(&catalog)
        .into_iter()
        .find(|option| option.name == name)
}

fn fallback_preset() -> ModelPreset {
    ModelPreset {
        backend: Backend::Auto,
        min_memory_gb: 0,
        max_memory_gb: u64::MAX,
        lighter: ModelOption {
            supported_os: Vec::new(),
            capabilities: serde_json::Value::Null,
            name: "HuggingFaceTB/SmolLM2-135M-Instruct".to_string(),
            label: "SmolLM2 135M".to_string(),
            notes: "fallback lighter preset".to_string(),
            source_kind: "huggingface-open".to_string(),
            source_url: "https://huggingface.co/HuggingFaceTB/SmolLM2-135M-Instruct/resolve/main/model.safetensors".to_string(),
            sha256: "5af571cbf074e6d21a03528d2330792e532ca608f24ac70a143f6b369968ab8c".to_string(),
            format: Some("gguf".to_string()),
            backend_compatibility: vec![Backend::Auto],
            estimated_vram_mb: Some(800),
        },
        recommended: ModelOption {
            supported_os: Vec::new(),
            capabilities: serde_json::Value::Null,
            name: "Qwen/Qwen2.5-0.5B-Instruct".to_string(),
            label: "Qwen 2.5 0.5B".to_string(),
            notes: "fallback recommended preset".to_string(),
            source_kind: "huggingface-open".to_string(),
            source_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/model.safetensors".to_string(),
            sha256: "fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe".to_string(),
            format: Some("gguf".to_string()),
            backend_compatibility: vec![Backend::Auto],
            estimated_vram_mb: Some(1200),
        },
    }
}

fn fallback_catalog() -> ModelCatalog {
    ModelCatalog {
        version: 1,
        presets: vec![fallback_preset()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_catalog() {
        let catalog = load_catalog().expect("catalog");
        assert_eq!(catalog.version, 1);
        assert!(!catalog.presets.is_empty());
    }

    #[test]
    fn chooses_mac_preset() {
        let selection = selection_for(Backend::M, 16);
        assert_eq!(selection.recommended.name, "Qwen/Qwen2.5-1.5B-Instruct");
    }

    #[test]
    fn looks_up_catalog_model() {
        let model = lookup_model("Qwen/Qwen2.5-1.5B-Instruct").expect("model");
        assert_eq!(model.name, "Qwen/Qwen2.5-1.5B-Instruct");
        assert_eq!(model.source_kind, "huggingface-open");
    }

    #[test]
    fn looks_up_backend_compatible_duplicate_catalog_model() {
        let model = lookup_model_for_backend("Qwen/Qwen2.5-1.5B-Instruct", Backend::M)
            .expect("Apple Silicon model");
        assert!(model.supports_backend(Backend::M));
        assert!(!model.supports_backend(Backend::Cuda));
    }

    #[test]
    fn apple_picker_includes_backend_compatible_duplicate_model() {
        let options = selectable_catalog_options_for(Backend::M, Some(6553));
        assert!(options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-1.5B-Instruct"));
    }

    #[test]
    fn apple_picker_scales_quantized_models_with_cap_applied_memory() {
        let eight_gb = selectable_catalog_options_for(Backend::M, Some(6553));
        assert!(eight_gb
            .iter()
            .any(|option| option.name == "mlx-community/Qwen2.5-3B-Instruct-4bit"));
        assert!(!eight_gb
            .iter()
            .any(|option| option.name == "mlx-community/Qwen2.5-7B-Instruct-4bit"));

        let sixty_four_gb = selectable_catalog_options_for(Backend::M, Some(52_428));
        assert!(sixty_four_gb
            .iter()
            .any(|option| option.name == "mlx-community/Qwen2.5-32B-Instruct-4bit"));
        assert!(!sixty_four_gb
            .iter()
            .any(|option| option.name == "mlx-community/Qwen2.5-72B-Instruct-4bit"));

        let one_twenty_eight_gb = selectable_catalog_options_for(Backend::M, Some(104_857));
        assert!(one_twenty_eight_gb
            .iter()
            .any(|option| option.name == "mlx-community/Qwen2.5-72B-Instruct-4bit"));
    }

    #[test]
    fn cuda_picker_scales_gguf_models_with_cap_applied_vram() {
        let eight_gb = selectable_catalog_options_for(Backend::Cuda, Some(6553));
        assert!(eight_gb
            .iter()
            .any(|option| option.name == "tensorblock/Qwen2.5-7B-Instruct-GGUF"));
        assert!(!eight_gb
            .iter()
            .any(|option| option.name == "tensorblock/Qwen2.5-14B-Instruct-GGUF"));

        let sixty_four_gb_at_eighty_percent =
            selectable_catalog_options_for(Backend::Cuda, Some(52_428));
        assert!(sixty_four_gb_at_eighty_percent
            .iter()
            .any(|option| option.name == "tensorblock/Qwen2.5-72B-Instruct-GGUF"));
    }

    #[test]
    fn vllm_picker_offers_native_hugging_face_models() {
        let options = selectable_catalog_options_for(Backend::Vllm, Some(36_000));

        assert!(options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-7B-Instruct-AWQ"));
        assert!(options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-32B-Instruct-AWQ"));
        assert!(options
            .iter()
            .all(|option| option.source_kind == "huggingface-vllm"));
        assert!(!options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-32B-Instruct"));
        assert!(!options
            .iter()
            .any(|option| option.name == "Qwen/Qwen3-Coder-30B-A3B-Instruct"));

        let high_cap_options = selectable_catalog_options_for(Backend::Vllm, Some(80_000));
        assert!(high_cap_options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-32B-Instruct"));
        assert!(high_cap_options
            .iter()
            .any(|option| option.name == "Qwen/Qwen3-Coder-30B-A3B-Instruct"));
        assert!(high_cap_options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-72B-Instruct-AWQ"));
        assert!(!high_cap_options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-72B-Instruct"));
    }

    #[test]
    fn muse_glimmer_is_an_optional_memory_gated_vllm_selection() {
        let name = crate::vllm_model_profile::MUSE_GLIMMER_MODEL;
        let options = selectable_catalog_options_for(Backend::Vllm, Some(73_728));
        assert_eq!(options.iter().filter(|option| option.name == name).count(), 1);
        for budget in [None, Some(73_727)] {
            assert!(!selectable_catalog_options_for(Backend::Vllm, budget).iter().any(|option| option.name == name));
        }
        for backend in [Backend::M, Backend::Cuda, Backend::Vulkan] {
            assert!(lookup_model_for_backend(name, backend).is_none());
        }
        assert!(lookup_model_for_backend(name, Backend::Vllm).is_some());
        assert_ne!(selection_for(Backend::Vllm, 128).recommended.name, name);
    }

    #[test]
    fn muse_fp8_and_bf16_have_separate_memory_budgets() {
        let fp8 = crate::vllm_model_profile::MUSE_GLIMMER_FP8_MODEL;
        let bf16 = crate::vllm_model_profile::MUSE_GLIMMER_MODEL;
        let options = selectable_catalog_options_for(Backend::Vllm, Some(49_152));
        assert_eq!(options.iter().filter(|option| option.name == fp8).count(), 1);
        assert!(!options.iter().any(|option| option.name == bf16));
        for budget in [None, Some(49_151)] {
            assert!(!selectable_catalog_options_for(Backend::Vllm, budget).iter().any(|option| option.name == fp8));
        }
        let large = selectable_catalog_options_for(Backend::Vllm, Some(100_000));
        for name in [fp8, bf16] {
            assert_eq!(large.iter().filter(|option| option.name == name).count(), 1);
            assert!(lookup_model_for_backend(name, Backend::Vllm).is_some());
            for backend in [Backend::M, Backend::Cuda, Backend::Vulkan] {
                assert!(lookup_model_for_backend(name, backend).is_none());
            }
        }
    }

    #[test]
    fn official_remote_catalog_entries_include_sha256() {
        let catalog = load_catalog().expect("catalog");
        for option in catalog
            .presets
            .iter()
            .flat_map(|preset| [&preset.lighter, &preset.recommended])
        {
            if option.source_kind == "huggingface-open" && !option.source_url.starts_with("file://")
            {
                assert_eq!(
                    option.sha256.len(),
                    64,
                    "{} should include a SHA-256 digest",
                    option.name
                );
                assert!(
                    option.sha256.chars().all(|ch| ch.is_ascii_hexdigit()),
                    "{} should use hexadecimal SHA-256",
                    option.name
                );
            }
        }
    }

    #[test]
    fn filters_cuda_catalog_by_cap_applied_vram_budget() {
        let catalog = load_catalog().expect("catalog");
        let options = options_for_machine(&catalog, Backend::Cuda, 16, Some(2048));

        assert!(!options.is_empty());
        assert!(options
            .iter()
            .all(|option| option.estimated_vram_mb.unwrap_or(u64::MAX) <= 2048));
        assert!(options
            .iter()
            .all(|option| option.supports_backend(Backend::Cuda)));
        assert!(options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-0.5B-Instruct"));
        assert!(!options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-1.5B-Instruct"));
    }

    #[test]
    fn selectable_catalog_options_include_all_unique_fitting_official_models() {
        let options = selectable_catalog_options_for(Backend::Cuda, Some(3276));

        assert!(options
            .iter()
            .any(|option| option.name == "HuggingFaceTB/SmolLM2-135M-Instruct"));
        assert!(options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-0.5B-Instruct"));
        assert!(options
            .iter()
            .any(|option| option.name == "Qwen/Qwen2.5-1.5B-Instruct"));
        assert!(options
            .iter()
            .any(|option| option.name == "tensorblock/Qwen2.5-3B-Instruct-GGUF"));
        assert!(!options
            .iter()
            .any(|option| option.name == "tensorblock/Qwen2.5-7B-Instruct-GGUF"));
        assert_eq!(
            options
                .iter()
                .filter(|option| option.name == "Qwen/Qwen2.5-0.5B-Instruct")
                .count(),
            1
        );
    }

    #[test]
    fn missing_vram_metadata_excludes_cuda_catalog_option() {
        let option_without_vram = ModelOption {
            supported_os: Vec::new(),
            capabilities: serde_json::Value::Null,
            name: "Test/MissingVram".to_string(),
            label: "Missing VRAM".to_string(),
            notes: "should not be offered for CUDA".to_string(),
            source_kind: "huggingface-open".to_string(),
            source_url: "file://missing.gguf".to_string(),
            sha256: String::new(),
            format: Some("gguf".to_string()),
            backend_compatibility: vec![Backend::Cuda],
            estimated_vram_mb: None,
        };
        let catalog = ModelCatalog {
            version: 1,
            presets: vec![ModelPreset {
                backend: Backend::Cuda,
                min_memory_gb: 0,
                max_memory_gb: 999,
                lighter: option_without_vram.clone(),
                recommended: option_without_vram,
            }],
        };

        let options = options_for_machine(&catalog, Backend::Cuda, 16, Some(4096));

        assert!(options.is_empty());
    }
}
