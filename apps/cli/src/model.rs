use crate::config::{config_dir, Config};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRecord {
    pub name: String,
    pub active: bool,
    pub cached_at: String,
    pub model_dir: String,
}

pub fn effective_model_dir(config: &Config) -> PathBuf {
    config
        .model_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir().join("models"))
}

pub fn ensure_effective_model_dir(config: &mut Config) -> PathBuf {
    let path = effective_model_dir(config);
    if config.model_dir.is_none() {
        config.model_dir = Some(path.display().to_string());
    }
    path
}

pub fn list_models(config: &Config) -> io::Result<Vec<ModelRecord>> {
    let dir = manifest_dir(config);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut models = vec![];
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read_to_string(entry.path())?;
        if let Ok(model) = serde_json::from_str::<ModelRecord>(&raw) {
            models.push(model);
        }
    }

    models.sort_by(|left, right| {
        right
            .active
            .cmp(&left.active)
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(models)
}

pub fn add_model(config: &mut Config, name: &str) -> io::Result<ModelRecord> {
    ensure_effective_model_dir(config);
    let mut models = list_models(config)?;
    let record = upsert_model(config, &mut models, name, false)?;
    sync_config_models(config, &models);
    write_models(config, &models)?;
    Ok(record)
}

pub fn use_model(config: &mut Config, name: &str) -> io::Result<ModelRecord> {
    ensure_effective_model_dir(config);
    let mut models = list_models(config)?;
    let record = upsert_model(config, &mut models, name, true)?;
    sync_config_models(config, &models);
    write_models(config, &models)?;
    Ok(record)
}

pub fn remove_model(config: &mut Config, name: &str, force: bool) -> io::Result<bool> {
    ensure_effective_model_dir(config);
    let mut models = list_models(config)?;
    let target_active = models.iter().any(|model| model.name == name && model.active);
    if target_active && !force {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to remove the active model; switch first or use --force",
        ));
    }

    let before = models.len();
    models.retain(|model| model.name != name);
    if before == models.len() {
        return Ok(false);
    }

    if target_active {
        config.active_model = None;
    }

    sync_config_models(config, &models);
    if models.is_empty() {
        remove_manifest_dir_if_empty(config)?;
    } else {
        write_models(config, &models)?;
    }

    Ok(true)
}

pub fn prune_models(config: &mut Config) -> io::Result<usize> {
    ensure_effective_model_dir(config);
    let mut models = list_models(config)?;
    let active_name = active_model_name(config);
    let before = models.len();

    models.retain(|model| match active_name.as_deref() {
        Some(name) => model.name == name,
        None => false,
    });

    let removed = before.saturating_sub(models.len());
    if models.is_empty() {
        remove_manifest_dir_if_empty(config)?;
    } else {
        write_models(config, &models)?;
    }
    sync_config_models(config, &models);
    Ok(removed)
}

pub fn active_model_name(config: &Config) -> Option<String> {
    if let Some(name) = config.active_model.clone() {
        return Some(name);
    }

    list_models(config)
        .ok()
        .and_then(|models| models.into_iter().find(|model| model.active).map(|model| model.name))
}

pub fn configured_model_dir_string(config: &Config) -> String {
    effective_model_dir(config).display().to_string()
}

pub fn manifest_dir(config: &Config) -> PathBuf {
    effective_model_dir(config).join(".opengpu")
}

fn now_unix_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn sanitize_model_name(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push('_');
        }
    }
    let trimmed = output.trim_matches('_');
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

fn manifest_path(config: &Config, name: &str) -> PathBuf {
    manifest_dir(config).join(format!("{}.json", sanitize_model_name(name)))
}

fn sync_config_models(config: &mut Config, models: &[ModelRecord]) {
    config.models = models.iter().map(|model| model.name.clone()).collect();
    config.active_model = models
        .iter()
        .find(|model| model.active)
        .map(|model| model.name.clone());
}

fn ensure_manifest_dir(config: &Config) -> io::Result<PathBuf> {
    let dir = manifest_dir(config);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn upsert_model(
    config: &mut Config,
    models: &mut Vec<ModelRecord>,
    name: &str,
    active: bool,
) -> io::Result<ModelRecord> {
    ensure_manifest_dir(config)?;
    let model_dir = effective_model_dir(config).display().to_string();
    let now = now_unix_seconds();

    for model in models.iter_mut() {
        if model.name == name {
            model.active = active || model.active;
            if active {
                model.active = true;
            }
            model.cached_at = now.clone();
            model.model_dir = model_dir.clone();
        } else if active {
            model.active = false;
        }
    }

    if !models.iter().any(|model| model.name == name) {
        models.push(ModelRecord {
            name: name.to_string(),
            active,
            cached_at: now,
            model_dir,
        });
    }

    if active {
        config.active_model = Some(name.to_string());
    } else if config.active_model.is_none() {
        config.active_model = models
            .iter()
            .find(|model| model.active)
            .map(|model| model.name.clone());
    }

    let record = models
        .iter()
        .find(|model| model.name == name)
        .cloned()
        .expect("model record");

    Ok(record)
}

fn write_models(config: &Config, models: &[ModelRecord]) -> io::Result<()> {
    let dir = ensure_manifest_dir(config)?;

    for model in models {
        let path = manifest_path(config, &model.name);
        let data = serde_json::to_string_pretty(model).expect("model serialization");
        fs::write(&path, format!("{data}\n"))?;
    }

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                let current_name = entry.file_name().to_string_lossy().to_string();
                let expected = models
                    .iter()
                    .map(|model| format!("{}.json", sanitize_model_name(&model.name)))
                    .collect::<Vec<_>>();
                if !expected.iter().any(|item| item == &current_name) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    Ok(())
}

fn remove_manifest_dir_if_empty(config: &Config) -> io::Result<()> {
    let dir = manifest_dir(config);
    if !dir.exists() {
        return Ok(());
    }

    if fs::read_dir(&dir)?.next().is_none() {
        let _ = fs::remove_dir(&dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn temp_config() -> (Config, PathBuf) {
        let temp_dir = std::env::temp_dir().join(format!(
            "opengpu-model-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut config = Config::default();
        config.model_dir = Some(temp_dir.display().to_string());
        (config, temp_dir)
    }

    #[test]
    fn sanitizes_model_names_for_cache_files() {
        assert_eq!(sanitize_model_name("llama3.1:8b"), "llama3_1_8b");
        assert_eq!(sanitize_model_name("CUDA/worker"), "cuda_worker");
    }

    #[test]
    fn add_use_remove_and_prune_models() {
        let (mut config, temp_dir) = temp_config();

        let added = add_model(&mut config, "llama3.1:8b").expect("add model");
        assert_eq!(added.name, "llama3.1:8b");
        assert!(config.models.contains(&"llama3.1:8b".to_string()));
        assert_eq!(config.active_model, None);

        let added_path = manifest_path(&config, "llama3.1:8b");
        assert!(added_path.exists());

        let active = use_model(&mut config, "llama3.1:8b").expect("use model");
        assert_eq!(active.name, "llama3.1:8b");
        assert_eq!(config.active_model.as_deref(), Some("llama3.1:8b"));

        let error = remove_model(&mut config, "llama3.1:8b", false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        add_model(&mut config, "llama3.2:3b").expect("add second model");
        let removed = prune_models(&mut config).expect("prune");
        assert_eq!(removed, 1);

        let models = list_models(&config).expect("list models");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "llama3.1:8b");
        assert!(models[0].active);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
