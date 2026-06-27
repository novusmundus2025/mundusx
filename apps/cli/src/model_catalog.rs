use crate::types::Backend;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
pub struct ModelOption {
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

#[derive(Clone, Debug, Deserialize)]
pub struct ModelPreset {
    pub backend: Backend,
    pub min_memory_gb: u64,
    pub max_memory_gb: u64,
    pub lighter: ModelOption,
    pub recommended: ModelOption,
}

#[derive(Clone, Debug, Deserialize)]
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
    if let Some(path) = std::env::var_os("OPENGPU_MODEL_CATALOG_PATH") {
        let raw = fs::read_to_string(PathBuf::from(path)).map_err(serde_json::Error::io)?;
        return serde_json::from_str(&raw);
    }

    serde_json::from_str(include_str!("../config/official-models.json"))
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
    let catalog = load_catalog().unwrap_or_else(|_| fallback_catalog());
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
    let preset = preset_for(catalog, backend, memory_gb);
    let mut options = Vec::new();

    for option in [preset.lighter, preset.recommended] {
        if options
            .iter()
            .any(|existing: &ModelOption| existing.name == option.name)
        {
            continue;
        }

        if !option.supports_backend(backend) {
            continue;
        }

        if backend == Backend::Cuda {
            let Some(estimated_vram_mb) = option.estimated_vram_mb else {
                continue;
            };
            let Some(available_vram_mb) = available_vram_mb else {
                continue;
            };
            if estimated_vram_mb > available_vram_mb {
                continue;
            }
        }

        options.push(option);
    }

    options
}

pub fn selectable_options_for(
    backend: Backend,
    memory_gb: u64,
    available_vram_mb: Option<u64>,
) -> Vec<ModelOption> {
    let catalog = load_catalog().unwrap_or_else(|_| fallback_catalog());
    options_for_machine(&catalog, backend, memory_gb, available_vram_mb)
}

pub fn lookup_model(name: &str) -> Option<ModelOption> {
    let catalog = load_catalog().ok()?;
    catalog
        .presets
        .iter()
        .flat_map(|preset| [preset.lighter.clone(), preset.recommended.clone()])
        .find(|option| option.name == name)
}

fn fallback_preset() -> ModelPreset {
    ModelPreset {
        backend: Backend::Auto,
        min_memory_gb: 0,
        max_memory_gb: u64::MAX,
        lighter: ModelOption {
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
    fn missing_vram_metadata_excludes_cuda_catalog_option() {
        let option_without_vram = ModelOption {
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
