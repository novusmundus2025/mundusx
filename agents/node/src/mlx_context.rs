//! Read cached model metadata only; never download during a heartbeat.
use crate::contracts::ModelCapability;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

const GIB: u64 = 1 << 30;

pub fn context_tokens(
    model: &ModelCapability,
    model_dir: &Path,
    physical_mb: u32,
    cap: u8,
    slots: u8,
) -> Option<u32> {
    let override_tokens = std::env::var("OPENGPU_MODEL_CONTEXT_TOKENS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n >= 1024);
    let directory = cached_directory(model, model_dir)?;
    let config: Value =
        serde_json::from_slice(&fs::read(directory.join("config.json")).ok()?).ok()?;
    let weights = weight_bytes(&directory).or(model.size_bytes)?;
    limit(&config, weights, physical_mb, cap, slots, override_tokens)
}

fn weight_bytes(directory: &Path) -> Option<u64> {
    let index = directory.join("model.safetensors.index.json");
    let paths = if index.exists() {
        let value: Value = serde_json::from_slice(&fs::read(index).ok()?).ok()?;
        let names: std::collections::BTreeSet<_> = value["weight_map"]
            .as_object()?
            .values()
            .map(|v| v.as_str())
            .collect::<Option<_>>()?;
        if names.is_empty()
            || names.iter().any(|name| {
                Path::new(name).components().count() != 1 || !name.ends_with(".safetensors")
            })
        {
            return None;
        }
        names
            .into_iter()
            .map(|name| directory.join(name))
            .collect::<Vec<_>>()
    } else {
        fs::read_dir(directory)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "safetensors"))
            .collect::<Vec<_>>()
    };
    // Hugging Face snapshot weights are symlinks to blobs. Stat the target,
    // not the short symlink itself, and require every indexed shard to exist.
    let sum = paths
        .into_iter()
        .try_fold(0u64, |sum, p| sum.checked_add(fs::metadata(p).ok()?.len()))?;
    (sum > 0).then_some(sum)
}

fn cached_directory(model: &ModelCapability, model_dir: &Path) -> Option<PathBuf> {
    for local in model
        .path
        .as_deref()
        .into_iter()
        .chain(std::iter::once(model.name.as_str()))
    {
        let p = PathBuf::from(local);
        if p.is_dir() && p.join("config.json").is_file() {
            return Some(p);
        }
    }
    let parts: Vec<_> = model.name.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return None;
    }
    let mut roots = Vec::new();
    for key in ["HF_HUB_CACHE", "HUGGINGFACE_HUB_CACHE"] {
        if let Some(p) = std::env::var_os(key) {
            roots.push(PathBuf::from(p));
        }
    }
    if let Some(p) = std::env::var_os("HF_HOME") {
        roots.push(PathBuf::from(p).join("hub"));
    }
    roots.push(model_dir.join(".huggingface/hub"));
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".cache/huggingface/hub"));
    }
    for root in roots {
        let repo = root.join(format!("models--{}", model.name.replace('/', "--")));
        let Some(revision) = fs::read_to_string(repo.join("refs/main")).ok() else {
            continue;
        };
        let revision = revision.trim();
        if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let snapshot = repo.join("snapshots").join(revision);
        if snapshot.join("config.json").is_file() {
            return Some(snapshot);
        }
    }
    None
}

fn limit(
    config: &Value,
    weights: u64,
    physical_mb: u32,
    cap: u8,
    slots: u8,
    requested: Option<u32>,
) -> Option<u32> {
    let c = config.get("text_config").unwrap_or(config);
    // These architectures use the standard dense K/V cache estimated below.
    // Unknown/hybrid architectures retain the existing conservative fallback.
    if !matches!(
        c["model_type"].as_str()?,
        "qwen3_moe" | "qwen3" | "qwen2" | "llama" | "mistral"
    ) {
        return None;
    }
    let maximum = c["max_position_embeddings"].as_u64()?.min(u32::MAX as u64);
    let layers = c["num_hidden_layers"].as_u64()?;
    let kv_heads = c["num_key_value_heads"]
        .as_u64()
        .or_else(|| c["num_attention_heads"].as_u64())?;
    let head_dim = c["head_dim"].as_u64().or_else(|| {
        c["hidden_size"]
            .as_u64()?
            .checked_div(c["num_attention_heads"].as_u64()?)
    })?;
    let element_bytes = match c["torch_dtype"].as_str() {
        Some("float32") => 4,
        Some("float16" | "bfloat16") | None => 2,
        _ => return None,
    };
    // K + V. Weight quantization does not imply KV quantization.
    let per_token = layers
        .checked_mul(kv_heads)?
        .checked_mul(head_dim)?
        .checked_mul(2 * element_bytes)?;
    if maximum == 0 || per_token == 0 || weights == 0 || physical_mb == 0 || cap == 0 {
        return None;
    }
    let budget = u64::from(physical_mb)
        .checked_mul(1 << 20)?
        .checked_mul(u64::from(cap.min(100)))?
        / 100;
    // Leave room for prefill activations, allocator overhead and retained prefix cache.
    let reserve = (4 * GIB).max(weights / 4);
    let remaining = budget.checked_sub(weights)?.checked_sub(reserve)?;
    let memory_limit = remaining / per_token.checked_mul(u64::from(slots.max(1)))?;
    let tokens = maximum
        .min(memory_limit)
        .min(u64::from(requested.unwrap_or(131_072)));
    let tokens = tokens / 1024 * 1024;
    (tokens >= 1024).then_some(tokens as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn qwen() -> Value {
        serde_json::json!({"model_type":"qwen3_moe","max_position_embeddings":262144,"num_hidden_layers":48,"num_key_value_heads":4,"head_dim":128})
    }
    #[test]
    fn large_mac_can_fit_an_agent_turn_without_ignoring_parallel_cache_memory() {
        let tokens = limit(&qwen(), 16 * GIB, 65536, 70, 3, None).unwrap();
        assert!(tokens >= 78443);
        assert!(tokens < limit(&qwen(), 16 * GIB, 65536, 70, 1, None).unwrap());
        assert!(limit(&qwen(), 16 * GIB, 32768, 70, 3, None).unwrap() < 78443);
    }
    #[test]
    fn override_is_clamped_to_model_and_memory_limits() {
        assert_eq!(
            limit(&qwen(), 16 * GIB, 65536, 70, 3, Some(81920)),
            Some(81920)
        );
        assert_eq!(
            limit(&qwen(), 16 * GIB, 65536, 70, 3, Some(u32::MAX)),
            limit(&qwen(), 16 * GIB, 65536, 70, 3, None)
        );
        let mut c = qwen();
        c["max_position_embeddings"] = serde_json::json!(32768);
        assert_eq!(limit(&c, 16 * GIB, 65536, 70, 1, Some(81920)), Some(32768));
        assert!(limit(&qwen(), 16 * GIB, 16384, 70, 1, None).is_none());
        assert!(limit(
            &serde_json::json!({"model_type":"unknown"}),
            16 * GIB,
            65536,
            70,
            1,
            None
        )
        .is_none());
    }
    #[test]
    fn reads_local_config_and_weights_without_network_or_name_guesses() {
        let dir = std::env::temp_dir().join(format!("mlx-context-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), qwen().to_string()).unwrap();
        fs::write(dir.join("weights.safetensors"), [1u8; 16]).unwrap();
        let model = ModelCapability {
            name: "arbitrary-name".into(),
            path: Some(dir.display().to_string()),
            ..Default::default()
        };
        assert!(context_tokens(&model, &dir, 65536, 70, 1).unwrap() > 16384);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn indexed_shards_must_all_exist_and_are_counted_once() {
        let dir = std::env::temp_dir().join(format!("mlx-shards-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("model.safetensors.index.json"),
            r#"{"weight_map":{"a":"one.safetensors","b":"one.safetensors","c":"two.safetensors"}}"#,
        )
        .unwrap();
        fs::write(dir.join("one.safetensors"), [0u8; 16]).unwrap();
        assert!(weight_bytes(&dir).is_none());
        fs::write(dir.join("two.safetensors"), [0u8; 32]).unwrap();
        assert_eq!(weight_bytes(&dir), Some(48));
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn snapshot_weights_use_blob_size_not_symlink_length() {
        let dir = std::env::temp_dir().join(format!("mlx-symlinks-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("blob"), [0u8; 4096]).unwrap();
        std::os::unix::fs::symlink(dir.join("blob"), dir.join("weights.safetensors")).unwrap();
        assert_eq!(weight_bytes(&dir), Some(4096));
        fs::remove_dir_all(dir).unwrap();
    }
}
