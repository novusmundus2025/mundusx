//! Built-in generation profiles for contributor-selected models. These do not
//! download models, change templates, or override explicit request settings.
use serde_json::{json, Value};

pub const MLX_GENERATE_SCRIPT: &str = r#"
import sys
from mlx_lm import load, generate
from mlx_lm.sample_utils import make_sampler, make_logits_processors
model, tokenizer = load(sys.argv[1])
prompt = tokenizer.apply_chat_template([{"role":"user","content":sys.argv[2]}], tokenize=False, add_generation_prompt=True)
print(generate(model, tokenizer, prompt=prompt, max_tokens=int(sys.argv[3]),
    sampler=make_sampler(temp=float(sys.argv[4]), top_p=float(sys.argv[5]), top_k=20),
    logits_processors=make_logits_processors(repetition_penalty=1.05), verbose=False))
"#;

pub fn is_qwen3_coder(model: &str) -> bool {
    let name = model.rsplit(['/', '\\']).next().unwrap_or(model).to_ascii_lowercase();
    name.starts_with("qwen3-coder-") && name.contains("instruct")
}

pub fn sampling(model: &str, temperature: Option<f32>, top_p: Option<f32>) -> (f32, f32) {
    let (default_temperature, default_top_p) = if is_qwen3_coder(model) { (0.7, 0.8) } else { (0.2, 0.9) };
    (temperature.unwrap_or(default_temperature).max(0.0), top_p.unwrap_or(default_top_p).clamp(0.0, 1.0))
}

pub fn verify_mlx_runtime(python: &std::path::Path) -> Result<(), String> {
    let result = std::process::Command::new(python).args(["-c", r#"
import inspect, mlx_lm.server
from mlx_lm.sample_utils import make_sampler, make_logits_processors
assert 'top_k' in inspect.signature(make_sampler).parameters
assert 'repetition_penalty' in inspect.signature(make_logits_processors).parameters
source = inspect.getsource(mlx_lm.server)
assert 'repetition_penalty' in source and 'top_k' in source
"#]).output().map_err(|e| format!("Could not verify MLX model-profile support: {e}"))?;
    if result.status.success() { Ok(()) } else {
        Err("Installed MLX runtime does not support the Qwen3-Coder generation profile. Update mlx-lm and restart the managed runtime.".into())
    }
}

pub fn apply(model: &str, payload: &mut Value) {
    if !is_qwen3_coder(model) { return; }
    for (key, value) in [("temperature", json!(0.7)), ("top_p", json!(0.8)), ("top_k", json!(20)), ("repetition_penalty", json!(1.05))] {
        if payload.get(key).is_none_or(Value::is_null) { payload[key] = value; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contributor_variants_select_profile_without_changing_model() {
        for model in ["mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit", "Qwen/Qwen3-Coder-30B-A3B-Instruct", "Qwen3-Coder-30B-A3B-Instruct-Q4_K_M.gguf"] {
            assert!(is_qwen3_coder(model));
            let mut payload = json!({"model":model}); apply(model, &mut payload);
            assert_eq!(payload["model"], model); assert_eq!(payload["top_k"], 20);
            assert_eq!(sampling(model, None, None), (0.7, 0.8));
        }
    }
    #[test]
    fn explicit_values_and_unknown_models_are_preserved() {
        let model = "Qwen/Qwen3-Coder-30B-A3B-Instruct";
        let mut payload = json!({"temperature":0,"top_p":1,"top_k":0,"repetition_penalty":1});
        let original = payload.clone(); apply(model, &mut payload); assert_eq!(payload, original);
        assert_eq!(sampling(model, Some(0.0), Some(1.0)), (0.0, 1.0));
        for model in ["Qwen/Qwen3-30B-A3B", "unknown", "not-qwen3-coder-instruct"] {
            let mut payload = json!({}); apply(model, &mut payload); assert_eq!(payload, json!({}));
        }
    }
}
