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
    sampler=make_sampler(temp=float(sys.argv[4]), top_p=float(sys.argv[5]), top_k=int(sys.argv[6])),
    logits_processors=make_logits_processors(repetition_penalty=float(sys.argv[7]), repetition_context_size=512), verbose=False))
"#;

pub fn is_qwen3_coder_next(model: &str) -> bool {
    let name = model.rsplit(['/', '\\']).next().unwrap_or(model).to_ascii_lowercase();
    name == "qwen3-coder-next" || ["4bit", "6bit", "8bit", "bf16", "fp16"].iter()
        .any(|suffix| name == format!("qwen3-coder-next-{suffix}"))
}

pub fn is_qwen3_coder(model: &str) -> bool {
    let name = model.rsplit(['/', '\\']).next().unwrap_or(model).to_ascii_lowercase();
    (name.starts_with("qwen3-coder-") && name.contains("instruct")) || is_qwen3_coder_next(model)
}

// Official Next model card/generation_config: temperature=1, top_p=.95, top_k=40.
// Keep its defaults separate from the older Coder Instruct profile.
pub fn top_k(model: &str) -> u32 { if is_qwen3_coder_next(model) {40} else {20} }
pub fn repetition_penalty(model: &str) -> f64 { if is_qwen3_coder_next(model) {1.0} else {1.05} }

fn default_sampling(model: &str) -> (f64,f64) {
    if is_qwen3_coder_next(model) {(1.0,0.95)}
    else if is_qwen3_coder(model) {(0.7,0.8)} else {(0.2,0.9)}
}

pub fn sampling(model: &str, temperature: Option<f32>, top_p: Option<f32>) -> (f32, f32) {
    let (default_temperature, default_top_p) = default_sampling(model);
    (temperature.unwrap_or(default_temperature as f32).max(0.0), top_p.unwrap_or(default_top_p as f32).clamp(0.0, 1.0))
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
    let (temperature,top_p)=default_sampling(model);
    for (key, value) in [("temperature", json!(temperature)), ("top_p", json!(top_p)), ("top_k", json!(top_k(model))), ("repetition_penalty", json!(repetition_penalty(model)))] {
        if payload.get(key).is_none_or(Value::is_null) { payload[key] = value; }
    }
}

/// MLX otherwise penalizes only the last 20 tokens, missing longer source loops.
/// Keep this runtime-specific option away from vLLM/llama OpenAI endpoints.
pub fn apply_mlx(model: &str, payload: &mut Value) {
    if is_qwen3_coder(model) { merge_leading_system_messages(payload); }
    let native_files = is_qwen3_coder(model) && payload["parallel_tool_calls"] != true
        && payload["tools"].as_array().is_some_and(|tools|
            ["read_file", "list_dir", "replace_string_in_file"].iter().all(|name|
                tools.iter().any(|tool|tool["function"]["name"].as_str()==Some(name))));
    if is_qwen3_coder(model) && payload.get("repetition_penalty").is_none_or(Value::is_null) {
        // Exact source edits must be able to copy observed identifiers and literals.
        // The native runtime guard independently stops actual argument loops.
        payload["repetition_penalty"] = json!(if native_files || is_qwen3_coder_next(model) {1.0} else {1.1});
    }
    apply(model, payload);
    if native_files {
        // A host may increase the conversation's answer budget to 8k or more.
        // Serial file operations must remain small enough to finish or fail
        // promptly. A 4k ceiling also permits legitimate exact-match route
        // extraction patches, whose old text counts against the output budget.
        payload["max_tokens"]=json!(payload["max_tokens"].as_u64().unwrap_or(4096).min(4096));
        if payload["chat_template_kwargs"].is_null() {payload["chat_template_kwargs"]=json!({});}
        payload["chat_template_kwargs"]["opengpu_native_tool_guard"]=json!(true);
    }
    if is_qwen3_coder(model) && payload.get("repetition_context_size").is_none_or(Value::is_null) {
        payload["repetition_context_size"] = json!(512);
    }
    // mlx-lm 0.32 ignores tool_choice. The managed wrapper enforces only the
    // required call opening; argument validation and execution remain upstream.
    if is_qwen3_coder(model) && payload["tool_choice"] == "required"
        && payload["tools"].as_array().is_some_and(|tools| !tools.is_empty()) {
        if payload["chat_template_kwargs"].is_null() { payload["chat_template_kwargs"] = json!({}); }
        if let Some(kwargs) = payload["chat_template_kwargs"].as_object_mut() {
            kwargs.insert("opengpu_require_tool_call".into(), json!(true));
        }
    }
}

// Qwen places its tool catalog after the first system message. Keep consecutive
// plain system instructions together so changing a fallback tool does not
// invalidate the much larger stable host-instruction prefix. Never move a
// message across user/assistant/tool history or discard named later messages.
fn merge_leading_system_messages(payload: &mut Value) {
    let Some(messages) = payload["messages"].as_array_mut() else { return; };
    let count = messages.iter().take_while(|message| message["role"] == "system"
        && message["content"].is_string()
        && message.as_object().is_some_and(|object| object.keys().all(|key| matches!(key.as_str(), "role" | "content" | "name")))).count();
    if count < 2 || messages[1..count].iter().any(|message| message.get("name").is_some()) { return; }
    let content = messages[..count].iter().map(|message| message["content"].as_str().unwrap()).collect::<Vec<_>>().join("\n\n");
    messages[0]["content"] = json!(content);
    messages.drain(1..count);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn next_uses_its_own_defaults_and_preserves_actual_client_sampling() {
        let model="mlx-community/Qwen3-Coder-Next-4bit";
        assert!(is_qwen3_coder(model));
        assert_eq!(sampling(model,None,None),(1.0,0.95));
        let mut payload=json!({"temperature":0.1,"top_p":1.0,"model":model});
        apply_mlx(model,&mut payload);
        assert_eq!(payload["temperature"],0.1);assert_eq!(payload["top_p"],1.0);
        assert_eq!(payload["top_k"],40);assert_eq!(payload["repetition_penalty"],1.0);
        assert_eq!(payload["model"],model);
        assert!(!is_qwen3_coder("unknown/Qwen3-Coder-Next-unverified-architecture"));
        let mut required=json!({"tool_choice":"required","tools":[{"type":"function"}]});
        apply_mlx(model,&mut required);
        assert_eq!(required["chat_template_kwargs"]["opengpu_require_tool_call"],true);
    }
    #[test]
    fn native_guard_is_limited_to_serial_vscode_coder_tools() {
        let tools=json!([{"function":{"name":"read_file"}},{"function":{"name":"list_dir"}},{"function":{"name":"replace_string_in_file"}}]);
        let mut payload=json!({"tools":tools,"temperature":0.1});
        apply_mlx("Qwen3-Coder-30B-A3B-Instruct-4bit",&mut payload);
        assert_eq!(payload["chat_template_kwargs"]["opengpu_native_tool_guard"],true);
        assert_eq!(payload["temperature"],0.1);
        assert_eq!(payload["repetition_penalty"],1.0);
        let mut explicit=json!({"tools":tools,"repetition_penalty":1.2});
        apply_mlx("Qwen3-Coder-30B-A3B-Instruct-4bit",&mut explicit);
        assert_eq!(explicit["repetition_penalty"],1.2);
        assert_eq!(payload["max_tokens"],4096);
        let mut large=json!({"tools":tools,"max_tokens":8192});
        apply_mlx("Qwen3-Coder-30B-A3B-Instruct-4bit",&mut large);
        assert_eq!(large["max_tokens"],4096);
        let mut small=json!({"tools":tools,"max_tokens":128});
        apply_mlx("Qwen3-Coder-30B-A3B-Instruct-4bit",&mut small);
        assert_eq!(small["max_tokens"],128);
        let mut parallel=json!({"tools":tools,"parallel_tool_calls":true});
        apply_mlx("Qwen3-Coder-30B-A3B-Instruct-4bit",&mut parallel);
        assert!(parallel["chat_template_kwargs"].get("opengpu_native_tool_guard").is_none());
        let mut other=json!({"tools":tools});apply_mlx("other",&mut other);
        assert!(other["chat_template_kwargs"].is_null());
    }
    #[test]
    fn mlx_preserves_instruction_order_and_keeps_dynamic_tools_after_stable_system_prefix() {
        let mut body = json!({"messages":[{"role":"system","name":"policy","content":"policy"},{"role":"system","content":"host"},{"role":"system","content":"harness"},{"role":"user","content":"request"},{"role":"system","content":"later"}],"tools":[{"type":"function","function":{"name":"read"}}]});
        let tools = body["tools"].clone();
        apply_mlx("Qwen3-Coder-30B-A3B-Instruct-4bit", &mut body);
        assert_eq!(body["messages"],json!([{"role":"system","name":"policy","content":"policy\n\nhost\n\nharness"},{"role":"user","content":"request"},{"role":"system","content":"later"}]));
        assert_eq!(body["tools"],tools);
        for messages in [json!([{"role":"system","content":[{"type":"text","text":"host"}]},{"role":"user","content":"request"}]),json!([{"role":"system","content":"first"},{"role":"system","name":"separate","content":"second"}])] {
            let mut payload=json!({"messages":messages});merge_leading_system_messages(&mut payload);assert_eq!(payload["messages"],messages);
        }
    }
    #[test]
    fn only_required_mlx_coder_tool_turns_get_the_runtime_requirement() {
        let model = "mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit";
        let body = json!({"tool_choice":"required","tools":[{"type":"function"}],"chat_template_kwargs":{"enable_thinking":false}});
        let mut required = body.clone(); apply_mlx(model, &mut required);
        assert_eq!(required["chat_template_kwargs"]["opengpu_require_tool_call"], true);
        assert_eq!(required["chat_template_kwargs"]["enable_thinking"], false);
        for choice in ["auto", "none"] {
            let mut optional = body.clone(); optional["tool_choice"] = json!(choice); apply_mlx(model,&mut optional);
            assert!(optional["chat_template_kwargs"].get("opengpu_require_tool_call").is_none());
        }
        let mut empty = body.clone(); empty["tools"] = json!([]); apply_mlx(model,&mut empty);
        assert!(empty["chat_template_kwargs"].get("opengpu_require_tool_call").is_none());
        let mut unknown = body.clone(); apply_mlx("other",&mut unknown);
        assert_eq!(unknown,body);
        let mut vllm = body; apply(model,&mut vllm);
        assert!(vllm["chat_template_kwargs"].get("opengpu_require_tool_call").is_none());
    }
    #[test]
    fn mlx_source_repetition_window_is_runtime_specific_and_overridable() {
        let model = "mlx-community/Qwen3-Coder-30B-A3B-Instruct-4bit";
        let mut mlx = json!({}); apply_mlx(model, &mut mlx);
        assert_eq!(mlx["repetition_context_size"], 512);
        assert_eq!(mlx["repetition_penalty"], 1.1);
        let mut explicit = json!({"repetition_context_size":64,"repetition_penalty":1.0}); apply_mlx(model, &mut explicit);
        assert_eq!(explicit["repetition_context_size"], 64);
        assert_eq!(explicit["repetition_penalty"], 1.0);
        let mut other_runtime = json!({}); apply(model, &mut other_runtime);
        assert!(other_runtime.get("repetition_context_size").is_none());
        let mut unknown = json!({}); apply_mlx("unknown", &mut unknown);
        assert_eq!(unknown, json!({}));
    }
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
