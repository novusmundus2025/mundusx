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

pub fn load_catalog() -> Result<ModelCatalog, serde_json::Error> {
    if let Some(path) = std::env::var_os("OPENGPU_MODEL_CATALOG_PATH") {
        let raw = fs::read_to_string(PathBuf::from(path)).map_err(serde_json::Error::io)?;
        return serde_json::from_str(&raw);
    }

    serde_json::from_str(include_str!("../config/official-models.json"))
}

pub fn selection_for(backend: Backend, memory_gb: u64) -> ModelSelection {
    let catalog = load_catalog().unwrap_or_else(|_| fallback_catalog());
    let _catalog_version = catalog.version;
    let preset = catalog
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
        .unwrap_or_else(|| fallback_preset());

    ModelSelection {
        backend: preset.backend,
        memory_gb,
        lighter: preset.lighter,
        recommended: preset.recommended,
    }
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
        },
        recommended: ModelOption {
            name: "Qwen/Qwen2.5-0.5B-Instruct".to_string(),
            label: "Qwen 2.5 0.5B".to_string(),
            notes: "fallback recommended preset".to_string(),
            source_kind: "huggingface-open".to_string(),
            source_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/model.safetensors".to_string(),
            sha256: "fdf756fa7fcbe7404d5c60e26bff1a0c8b8aa1f72ced49e7dd0210fe288fb7fe".to_string(),
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
}
