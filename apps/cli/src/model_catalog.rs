use crate::types::Backend;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct ModelOption {
    pub name: String,
    pub label: String,
    pub notes: String,
    pub source_kind: String,
    pub source_path: String,
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
        .unwrap_or_else(|| ModelPreset {
            backend: Backend::Auto,
            min_memory_gb: 0,
            max_memory_gb: u64::MAX,
            lighter: ModelOption {
                name: "llama3.2:1b".to_string(),
                label: "Llama 3.2 1B".to_string(),
                notes: "fallback lighter preset".to_string(),
                source_kind: "repo-file".to_string(),
                source_path: "model-artifacts/llama3.2-1b.asset".to_string(),
            },
            recommended: ModelOption {
                name: "llama3.2:3b".to_string(),
                label: "Llama 3.2 3B".to_string(),
                notes: "fallback recommended preset".to_string(),
                source_kind: "repo-file".to_string(),
                source_path: "model-artifacts/llama3.2-3b.asset".to_string(),
            },
        });

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

pub fn artifact_bytes_for(source_path: &str) -> Option<&'static [u8]> {
    match source_path {
        "model-artifacts/llama3.2-1b.asset" => Some(include_bytes!("../model-artifacts/llama3.2-1b.asset")),
        "model-artifacts/llama3.2-3b.asset" => Some(include_bytes!("../model-artifacts/llama3.2-3b.asset")),
        "model-artifacts/llama3.1-8b.asset" => Some(include_bytes!("../model-artifacts/llama3.1-8b.asset")),
        "model-artifacts/llama3.1-8b-q4.asset" => Some(include_bytes!("../model-artifacts/llama3.1-8b-q4.asset")),
        "model-artifacts/llama3.3-70b-q4.asset" => {
            Some(include_bytes!("../model-artifacts/llama3.3-70b-q4.asset"))
        }
        _ => None,
    }
}

fn fallback_catalog() -> ModelCatalog {
    ModelCatalog {
        version: 1,
        presets: vec![ModelPreset {
            backend: Backend::Auto,
            min_memory_gb: 0,
            max_memory_gb: u64::MAX,
            lighter: ModelOption {
                name: "llama3.2:1b".to_string(),
                label: "Llama 3.2 1B".to_string(),
                notes: "fallback lighter preset".to_string(),
                source_kind: "repo-file".to_string(),
                source_path: "model-artifacts/llama3.2-1b.asset".to_string(),
            },
            recommended: ModelOption {
                name: "llama3.2:3b".to_string(),
                label: "Llama 3.2 3B".to_string(),
                notes: "fallback recommended preset".to_string(),
                source_kind: "repo-file".to_string(),
                source_path: "model-artifacts/llama3.2-3b.asset".to_string(),
            },
        }],
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
        assert_eq!(selection.recommended.name, "llama3.1:8b");
    }

    #[test]
    fn looks_up_catalog_model() {
        let model = lookup_model("llama3.1:8b").expect("model");
        assert_eq!(model.name, "llama3.1:8b");
        assert_eq!(model.source_kind, "repo-file");
    }

    #[test]
    fn resolves_artifact_bytes() {
        let bytes = artifact_bytes_for("model-artifacts/llama3.1-8b.asset").expect("bytes");
        assert!(!bytes.is_empty());
    }
}
