//! Exclusions for unsupported contributed runtimes.
pub fn unsupported_runtime(kind: &str, _base_url: &str) -> bool {
    let normalized: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    normalized.contains("nvpair") || normalized.contains("nvidiapair")
}

/// LM Studio's native listing distinguishes downloaded files from loaded LLMs.
/// Return None for an unrelated or unsupported response, including Ollama.
pub fn lmstudio_loaded_listing(body: &serde_json::Value) -> Option<serde_json::Value> {
    if body.get("error").is_some() { return None; }
    if let Some(models) = body.get("models").and_then(serde_json::Value::as_array) {
        if !models.iter().all(|m| m.get("key").and_then(serde_json::Value::as_str).is_some()
            && m.get("loaded_instances").and_then(serde_json::Value::as_array).is_some()) { return None; }
        let mut data = Vec::new();
        for model in models.iter().filter(|m| m["type"] == "llm") {
            for instance in model["loaded_instances"].as_array()? {
                if let Some(id) = instance["id"].as_str().filter(|id| !id.is_empty()) {
                    data.push(serde_json::json!({"id": id, "owned_by": "lmstudio",
                        "context_length": instance["config"]["context_length"]}));
                }
            }
        }
        return Some(serde_json::json!({"data": data}));
    }
    let models = body.get("data")?.as_array()?;
    if !models.iter().all(|m| m.get("id").and_then(serde_json::Value::as_str).is_some()
        && matches!(m["state"].as_str(), Some("loaded" | "not-loaded"))) { return None; }
    Some(serde_json::json!({"data": models.iter()
        .filter(|m| m["state"] == "loaded" && m["type"] != "embeddings" && m["type"] != "embedding")
        .cloned().collect::<Vec<_>>()}))
}

pub fn listing_contains(body: &serde_json::Value, native_ollama: bool, model: &str) -> bool {
    valid_model_listing(body, native_ollama) && body[if native_ollama { "models" } else { "data" }]
        .as_array().is_some_and(|entries| entries.iter().any(|entry| {
            let id = if native_ollama { entry.get("name").or_else(|| entry.get("model")) } else { entry.get("id") };
            id.and_then(serde_json::Value::as_str) == Some(model)
        }))
}

pub fn unsupported_listing(body: &serde_json::Value) -> bool {
    body.get("data")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("owned_by")
                    .and_then(serde_json::Value::as_str)
                    .map(|owner| unsupported_runtime(owner, ""))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Accept protocol model listings, not arbitrary JSON from another service.
pub fn valid_model_listing(body: &serde_json::Value, native_ollama: bool) -> bool {
    if body.get("error").is_some() { return false; }
    let field = if native_ollama { "models" } else { "data" };
    let Some(entries) = body.get(field).and_then(serde_json::Value::as_array) else { return false; };
    entries.iter().all(|entry| {
        let name = if native_ollama { entry.get("name").or_else(|| entry.get("model")) } else { entry.get("id") };
        name.and_then(serde_json::Value::as_str).is_some_and(|value| !value.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loaded_instances_exclude_downloads_and_embeddings() {
        let body = serde_json::json!({"models": [
            {"key":"download", "type":"llm", "loaded_instances":[]},
            {"key":"file", "type":"llm", "loaded_instances":[{"id":"chosen", "config":{"context_length":8192}}]},
            {"key":"embed", "type":"embedding", "loaded_instances":[{"id":"embedding"}]}
        ]});
        let listing = lmstudio_loaded_listing(&body).unwrap();
        assert!(listing_contains(&listing, false, "chosen"));
        assert!(!listing_contains(&listing, false, "file"));
        assert!(!listing_contains(&listing, false, "download"));
        assert!(!listing_contains(&listing, false, "embedding"));
        assert!(lmstudio_loaded_listing(&serde_json::json!({"models":[{"name":"ollama"}]})).is_none());
        assert!(lmstudio_loaded_listing(&serde_json::json!({"error":"unauthorized"})).is_none());
    }

    #[test]
    fn legacy_loaded_state_and_exact_model_are_required() {
        let listing = lmstudio_loaded_listing(&serde_json::json!({"data":[
            {"id":"a", "state":"loaded", "type":"llm"},
            {"id":"b", "state":"not-loaded", "type":"llm"}
        ]})).unwrap();
        assert!(listing_contains(&listing, false, "a"));
        assert!(!listing_contains(&listing, false, "b"));
        assert!(!listing_contains(&listing, false, "A"));
    }
    #[test]
    fn permits_lmstudio_but_blocks_unaccounted_pair_capacity() {
        for kind in ["lm-studio", "LM Studio", "lm_studio", "lmstudio"] {
            assert!(!unsupported_runtime(kind, "http://localhost:9999"));
        }
        assert!(!unsupported_runtime(
            "openai-compatible",
            "http://[::1]:1234/v1"
        ));
        assert!(!unsupported_runtime("vllm", "http://localhost:8000"));
        assert!(!unsupported_runtime("ollama", "http://localhost:11434"));
        assert!(unsupported_runtime("nvidia-pair", "http://localhost:11434"));
    }
    #[test]
    fn permits_supported_custom_port_servers() {
        assert!(!unsupported_listing(
            &serde_json::json!({"data":[{"owned_by":"lmstudio"}]})
        ));
        assert!(!unsupported_listing(
            &serde_json::json!({"data":[{"owned_by":"vllm"}]})
        ));
    }
}
