use crate::config::{config_dir, Config};
use crate::model_catalog::{lookup_model, ModelOption};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRecord {
    pub name: String,
    pub active: bool,
    pub cached_at: String,
    pub model_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_vram_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ImportModelOptions<'a> {
    pub name: Option<&'a str>,
    pub path: &'a Path,
    pub active: bool,
    pub backend: crate::types::Backend,
    pub available_vram_mb: Option<u64>,
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
    let _ = download_model_if_available(config, name)?;
    let mut models = list_models(config)?;
    let record = upsert_model(config, &mut models, name, false)?;
    sync_config_models(config, &models);
    write_models(config, &models)?;
    Ok(record)
}

pub fn use_model(config: &mut Config, name: &str) -> io::Result<ModelRecord> {
    ensure_effective_model_dir(config);
    let _ = download_model_if_available(config, name)?;
    let cached_path = cached_model_path(config, name)?;
    if cached_path.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("model `{name}` is not cached; download or import it before activating"),
        ));
    }
    let mut models = list_models(config)?;
    let record = upsert_model(config, &mut models, name, true)?;
    sync_config_models(config, &models);
    write_models(config, &models)?;
    Ok(record)
}

pub fn import_model(
    config: &mut Config,
    options: ImportModelOptions<'_>,
) -> io::Result<ModelRecord> {
    ensure_effective_model_dir(config);
    let source = options.path;
    let metadata = fs::metadata(source)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "model import path must point to a file",
        ));
    }

    let name = options
        .name
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| {
            source
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "model name is required"))?;

    let mut models = list_models(config)?;
    let record = upsert_imported_model(config, &mut models, &name, options, metadata.len())?;
    sync_config_models(config, &models);
    write_models(config, &models)?;
    Ok(record)
}

pub fn ensure_catalog_model_fits(
    name: &str,
    backend: crate::types::Backend,
    available_vram_mb: Option<u64>,
) -> io::Result<()> {
    let Some(option) = lookup_model(name) else {
        return Ok(());
    };

    if option.format.as_deref() != Some("gguf") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to download `{name}`: catalog entry is not a GGUF model"),
        ));
    }

    if backend != crate::types::Backend::Cuda {
        if !option.supports_backend(backend) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to download `{name}`: catalog entry is not compatible with {backend}"
                ),
            ));
        }
        return Ok(());
    }

    if !option.supports_backend(backend) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to download `{name}`: catalog entry is not compatible with {backend}"
            ),
        ));
    }

    let Some(estimated_vram_mb) = option.estimated_vram_mb else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to download `{name}`: catalog is missing estimated VRAM metadata"),
        ));
    };

    let Some(available_vram_mb) = available_vram_mb else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("refusing to download `{name}`: CUDA VRAM could not be detected"),
        ));
    };

    if estimated_vram_mb > available_vram_mb {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to download `{name}`: estimated {estimated_vram_mb} MB VRAM exceeds available budget {available_vram_mb} MB"
            ),
        ));
    }

    Ok(())
}

pub fn remove_model(config: &mut Config, name: &str, force: bool) -> io::Result<bool> {
    ensure_effective_model_dir(config);
    let mut models = list_models(config)?;
    let target_active = models
        .iter()
        .any(|model| model.name == name && model.active);
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
    let _ = remove_model_dir(config, name);
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
    let active_to_keep = active_name.as_deref();
    for model in models.iter() {
        if Some(model.name.as_str()) != active_to_keep {
            let _ = remove_model_dir(config, &model.name);
        }
    }
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

    list_models(config).ok().and_then(|models| {
        models
            .into_iter()
            .find(|model| model.active)
            .map(|model| model.name)
    })
}

pub fn configured_model_dir_string(config: &Config) -> String {
    effective_model_dir(config).display().to_string()
}

pub fn manifest_dir(config: &Config) -> PathBuf {
    effective_model_dir(config).join(".opengpu")
}

fn model_cache_dir(config: &Config, name: &str) -> PathBuf {
    effective_model_dir(config).join(sanitize_model_name(name))
}

fn source_filename(source_url: &str) -> String {
    let without_query = source_url.split('?').next().unwrap_or(source_url);
    Path::new(without_query)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("model.safetensors")
        .to_string()
}

fn model_file_path(config: &Config, name: &str, option: &ModelOption) -> PathBuf {
    model_cache_dir(config, name).join(source_filename(&option.source_url))
}

fn cached_model_path(config: &Config, name: &str) -> io::Result<Option<PathBuf>> {
    if let Some(option) = lookup_model(name) {
        let path = model_file_path(config, name, &option);
        if path.is_file() {
            return Ok(Some(path));
        }
    }

    for model in list_models(config)? {
        if model.name != name {
            continue;
        }

        if let Some(path) = model
            .source_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.is_file())
        {
            return Ok(Some(path));
        }

        if let Some(path) = model.file_name.as_ref().map(|file_name| {
            effective_model_dir(config)
                .join(sanitize_model_name(&model.name))
                .join(file_name)
        }) {
            if path.is_file() {
                return Ok(Some(path));
            }
        }
    }

    let cache_dir = model_cache_dir(config, name);
    if cache_dir.exists() {
        let mut files = Vec::new();
        collect_gguf_files(&cache_dir, &mut files)?;
        files.sort();
        return Ok(files.into_iter().next());
    }

    Ok(None)
}

fn collect_gguf_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_gguf_files(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("gguf") {
            files.push(path);
        }
    }

    Ok(())
}

fn model_format(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
}

fn model_quantization(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let parts = stem.split(['.', '-']).collect::<Vec<_>>();
    for part in parts {
        let subparts = part.split('_').collect::<Vec<_>>();
        for (index, value) in subparts.iter().enumerate() {
            if value.len() >= 2
                && value.starts_with('q')
                && value[1..].chars().all(|ch| ch.is_ascii_digit())
            {
                let mut tag = vec![*value];
                for next in subparts.iter().skip(index + 1).take(2) {
                    if next.len() == 1 && next.chars().all(|ch| ch.is_ascii_alphabetic()) {
                        tag.push(*next);
                    } else {
                        break;
                    }
                }
                return Some(tag.join("_").to_ascii_uppercase());
            }
        }
    }
    None
}

fn estimate_vram_mb(size_bytes: u64) -> u64 {
    let base_mb = size_bytes.div_ceil(1024 * 1024);
    base_mb.saturating_mul(5).div_ceil(4).saturating_add(512)
}

fn compatibility_for(
    backend: crate::types::Backend,
    format: Option<&str>,
    estimated_vram_mb: u64,
    available_vram_mb: Option<u64>,
) -> (String, String) {
    if format != Some("gguf") {
        return (
            "rejected".to_string(),
            "only GGUF files are currently runnable by the local worker".to_string(),
        );
    }

    if backend == crate::types::Backend::Cuda {
        match available_vram_mb {
            Some(available) if estimated_vram_mb > available => (
                "rejected".to_string(),
                format!("estimated {estimated_vram_mb} MB VRAM exceeds available {available} MB"),
            ),
            Some(available) if estimated_vram_mb > available.saturating_mul(4) / 5 => (
                "degraded".to_string(),
                format!(
                    "estimated {estimated_vram_mb} MB VRAM is close to available {available} MB"
                ),
            ),
            Some(available) => (
                "accepted".to_string(),
                format!("estimated {estimated_vram_mb} MB VRAM fits available {available} MB"),
            ),
            None => (
                "degraded".to_string(),
                "CUDA VRAM was not supplied; compatibility needs runtime confirmation".to_string(),
            ),
        }
    } else {
        (
            "accepted".to_string(),
            "GGUF file is compatible with the local llama.cpp worker path".to_string(),
        )
    }
}

fn download_model_if_available(config: &Config, name: &str) -> io::Result<bool> {
    let Some(option) = lookup_model(name) else {
        return Ok(false);
    };

    download_model_from_option(config, name, &option)
}

fn download_model_from_option(
    config: &Config,
    name: &str,
    option: &ModelOption,
) -> io::Result<bool> {
    if option.source_kind != "huggingface-open" {
        return Ok(false);
    }

    let dest = model_file_path(config, name, &option);
    if dest.exists() {
        return Ok(false);
    }

    let is_local_source = option.source_url.starts_with("file://");
    if !is_local_source && option.sha256.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("refusing to download `{name}`: official remote model is missing sha256"),
        ));
    }

    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let tmp = dest.with_extension("downloading");
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }

    if is_local_source {
        let source_path = option.source_url.trim_start_matches("file://");
        fs::copy(source_path, &tmp)?;
    } else {
        let status = Command::new("curl")
            .args([
                "-fL",
                "--retry",
                "3",
                "--continue-at",
                "-",
                "--silent",
                "--show-error",
                "--output",
            ])
            .arg(&tmp)
            .arg(&option.source_url)
            .status()?;

        if !status.success() {
            let _ = fs::remove_file(&tmp);
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("failed to download model from {}", option.source_url),
            ));
        }
    }

    if !option.sha256.trim().is_empty() {
        verify_sha256(&tmp, &option.sha256)?;
    }

    if dest.exists() {
        let _ = fs::remove_file(&dest);
    }
    fs::rename(&tmp, &dest)?;
    Ok(true)
}

fn verify_sha256(path: &Path, expected: &str) -> io::Result<()> {
    let expected = expected.trim();
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = format!("{:x}", hasher.finalize());
    if digest.eq_ignore_ascii_case(expected) {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("checksum mismatch for {}", path.display()),
    ))
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
            source_path: None,
            file_name: None,
            format: None,
            quantization: None,
            size_bytes: None,
            estimated_vram_mb: None,
            compatibility: None,
            compatibility_reason: None,
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

fn upsert_imported_model(
    config: &mut Config,
    models: &mut Vec<ModelRecord>,
    name: &str,
    options: ImportModelOptions<'_>,
    size_bytes: u64,
) -> io::Result<ModelRecord> {
    ensure_manifest_dir(config)?;
    let model_dir = effective_model_dir(config).display().to_string();
    let now = now_unix_seconds();
    let format = model_format(options.path);
    let estimated_vram_mb = estimate_vram_mb(size_bytes);
    let (compatibility, compatibility_reason) = compatibility_for(
        options.backend,
        format.as_deref(),
        estimated_vram_mb,
        options.available_vram_mb,
    );
    let active = options.active && compatibility != "rejected";

    for model in models.iter_mut() {
        if model.name == name {
            model.active = if compatibility == "rejected" {
                false
            } else {
                active || model.active
            };
            model.cached_at = now.clone();
            model.model_dir = model_dir.clone();
            model.source_path = Some(options.path.display().to_string());
            model.file_name = options
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string());
            model.format = format.clone();
            model.quantization = model_quantization(options.path);
            model.size_bytes = Some(size_bytes);
            model.estimated_vram_mb = Some(estimated_vram_mb);
            model.compatibility = Some(compatibility.clone());
            model.compatibility_reason = Some(compatibility_reason.clone());
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
            source_path: Some(options.path.display().to_string()),
            file_name: options
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string()),
            format,
            quantization: model_quantization(options.path),
            size_bytes: Some(size_bytes),
            estimated_vram_mb: Some(estimated_vram_mb),
            compatibility: Some(compatibility),
            compatibility_reason: Some(compatibility_reason),
        });
    }

    if active {
        config.active_model = Some(name.to_string());
    }

    models
        .iter()
        .find(|model| model.name == name)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "model import failed"))
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

fn remove_model_dir(config: &Config, name: &str) -> io::Result<()> {
    let dir = model_cache_dir(config, name);
    if dir.exists() {
        fs::remove_dir_all(dir)?;
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
        assert_eq!(sanitize_model_name("model/worker"), "model_worker");
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

        let cache_dir = model_cache_dir(&config, "llama3.1:8b");
        fs::create_dir_all(&cache_dir).expect("cache dir");
        fs::write(cache_dir.join("llama3.1-q4_k_m.gguf"), b"model").expect("model");

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

    #[test]
    fn use_model_rejects_manifest_without_cached_file() {
        let (mut config, temp_dir) = temp_config();

        add_model(&mut config, "Missing/Model").expect("add manifest");
        let error = use_model(&mut config, "Missing/Model")
            .expect_err("missing model file should not activate");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(config.active_model, None);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn downloads_local_file_source_for_open_model() {
        let (config, temp_dir) = temp_config();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let source_path = temp_dir.join("source.safetensors");
        fs::write(&source_path, b"model-bytes").expect("write source");

        let option = crate::model_catalog::ModelOption {
            name: "Test/OpenModel".to_string(),
            label: "Test Open Model".to_string(),
            notes: "local test source".to_string(),
            source_kind: "huggingface-open".to_string(),
            source_url: format!("file://{}", source_path.display()),
            sha256: String::new(),
            format: Some("gguf".to_string()),
            backend_compatibility: vec![crate::types::Backend::Auto],
            estimated_vram_mb: Some(1),
        };

        let downloaded =
            download_model_from_option(&config, &option.name, &option).expect("download");
        assert!(downloaded);

        let dest = model_file_path(&config, &option.name, &option);
        assert!(dest.exists());
        assert_eq!(fs::read(&dest).expect("dest"), b"model-bytes");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn verifies_sha256_without_external_tools() {
        let (_config, temp_dir) = temp_config();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let path = temp_dir.join("checksum.gguf");
        fs::write(&path, b"model").expect("model");

        verify_sha256(
            &path,
            "9372c470eeadd5ecd9c3c74c2b3cb633f8e2f2fad799250a0f70d652b6b825e4",
        )
        .expect("checksum should verify");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn rejects_sha256_mismatch_without_external_tools() {
        let (_config, temp_dir) = temp_config();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let path = temp_dir.join("checksum-mismatch.gguf");
        fs::write(&path, b"model").expect("model");

        let error = verify_sha256(
            &path,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect_err("checksum should fail");

        assert!(error.to_string().contains("checksum mismatch"));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn rejects_remote_open_model_without_checksum_before_download() {
        let (config, temp_dir) = temp_config();

        let option = crate::model_catalog::ModelOption {
            name: "Test/RemoteModel".to_string(),
            label: "Test Remote Model".to_string(),
            notes: "remote test source".to_string(),
            source_kind: "huggingface-open".to_string(),
            source_url: "https://example.invalid/model.gguf".to_string(),
            sha256: String::new(),
            format: Some("gguf".to_string()),
            backend_compatibility: vec![crate::types::Backend::Auto],
            estimated_vram_mb: Some(1),
        };

        let error = download_model_from_option(&config, &option.name, &option).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("missing sha256"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn rejects_catalog_model_before_download_when_cuda_vram_is_too_small() {
        let error = ensure_catalog_model_fits(
            "Qwen/Qwen2.5-1.5B-Instruct",
            crate::types::Backend::Cuda,
            Some(512),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("refusing to download"));
    }

    #[test]
    fn accepts_catalog_model_before_download_when_cuda_vram_fits() {
        ensure_catalog_model_fits(
            "Qwen/Qwen2.5-0.5B-Instruct",
            crate::types::Backend::Cuda,
            Some(4096),
        )
        .expect("model should fit");
    }

    #[test]
    fn imports_compatible_local_gguf_model() {
        let (mut config, temp_dir) = temp_config();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let source_path = temp_dir.join("llama-3.2-q4_k_m.gguf");
        fs::write(&source_path, vec![0u8; 1024 * 1024]).expect("write source");

        let record = import_model(
            &mut config,
            ImportModelOptions {
                name: Some("local-llama"),
                path: &source_path,
                active: true,
                backend: crate::types::Backend::Cuda,
                available_vram_mb: Some(4096),
            },
        )
        .expect("import model");

        assert_eq!(record.name, "local-llama");
        assert!(record.active);
        assert_eq!(record.format.as_deref(), Some("gguf"));
        assert_eq!(record.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(record.compatibility.as_deref(), Some("accepted"));
        assert_eq!(config.active_model.as_deref(), Some("local-llama"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn rejects_local_model_that_exceeds_cuda_vram() {
        let (mut config, temp_dir) = temp_config();
        fs::create_dir_all(&temp_dir).expect("temp dir");
        let source_path = temp_dir.join("too-large-q8_0.gguf");
        fs::write(&source_path, vec![0u8; 2 * 1024 * 1024]).expect("write source");

        let record = import_model(
            &mut config,
            ImportModelOptions {
                name: None,
                path: &source_path,
                active: true,
                backend: crate::types::Backend::Cuda,
                available_vram_mb: Some(1),
            },
        )
        .expect("import model");

        assert_eq!(record.name, "too-large-q8_0");
        assert!(!record.active);
        assert_eq!(config.active_model, None);
        assert_eq!(record.compatibility.as_deref(), Some("rejected"));
        assert!(record
            .compatibility_reason
            .as_deref()
            .unwrap_or("")
            .contains("exceeds available"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn import_requires_existing_model_file() {
        let (mut config, temp_dir) = temp_config();
        let missing = temp_dir.join("missing.gguf");

        let error = import_model(
            &mut config,
            ImportModelOptions {
                name: Some("missing"),
                path: &missing,
                active: false,
                backend: crate::types::Backend::Cuda,
                available_vram_mb: Some(4096),
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
