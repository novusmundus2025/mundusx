//! Exclusions for unsupported contributed runtimes.
pub fn unsupported_runtime(kind: &str, base_url: &str) -> bool {
    let normalized: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    normalized.contains("lmstudio")
        || base_url
            .trim_end_matches('/')
            .split('/')
            .nth(2)
            .unwrap_or("")
            .rsplit(':')
            .next()
            == Some("1234")
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_legacy_configs_and_default_port() {
        for kind in ["lm-studio", "LM Studio", "lm_studio", "lmstudio"] {
            assert!(unsupported_runtime(kind, "http://localhost:9999"));
        }
        assert!(unsupported_runtime(
            "openai-compatible",
            "http://[::1]:1234/v1"
        ));
        assert!(!unsupported_runtime("vllm", "http://localhost:8000"));
        assert!(!unsupported_runtime("ollama", "http://localhost:11434"));
    }
    #[test]
    fn excludes_identified_custom_port_servers() {
        assert!(unsupported_listing(
            &serde_json::json!({"data":[{"owned_by":"lmstudio"}]})
        ));
        assert!(!unsupported_listing(
            &serde_json::json!({"data":[{"owned_by":"vllm"}]})
        ));
    }
}
