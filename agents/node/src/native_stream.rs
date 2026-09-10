//! OpenAI message deltas, kept separate from ordinary assistant text.
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct NativeStream {
    content: String,
    calls: BTreeMap<usize, Value>,
}
fn append(target: &mut Value, part: &Value) -> Result<(), String> {
    if part.is_null() { return Ok(()); }
    let text = part.as_str().ok_or("stream field must be a string")?;
    let mut joined = target.as_str().unwrap_or_default().to_owned();
    joined.push_str(text);
    *target = Value::String(joined);
    Ok(())
}
impl NativeStream {
    pub fn push(&mut self, delta: &Value) -> Result<(), String> {
        if !delta.is_object() { return Err("native delta must be an object".into()); }
        if let Some(content) = delta.get("content").filter(|v| !v.is_null()) {
            self.content.push_str(content.as_str().ok_or("content must be a string")?);
        }
        if let Some(calls) = delta.get("tool_calls") {
            for call in calls.as_array().ok_or("tool_calls must be an array")? {
                let index = call["index"].as_u64().filter(|v| *v < 128).ok_or("invalid tool index")? as usize;
                let entry = self.calls.entry(index).or_insert_with(|| json!({"id":"", "type":"function", "function":{"name":"","arguments":""}}));
                append(&mut entry["id"], &call["id"])?;
                append(&mut entry["function"]["name"], &call["function"]["name"])?;
                append(&mut entry["function"]["arguments"], &call["function"]["arguments"])?;
            }
        }
        Ok(())
    }
    pub fn message(&self) -> Value {
        let mut message = json!({"role":"assistant", "content":self.content});
        if !self.calls.is_empty() { message["tool_calls"] = self.calls.values().cloned().collect(); }
        message
    }
    // The relay may stop delivering midway. Append only verified missing suffixes.
    pub fn remainder(&self, final_message: &Value) -> Result<Value, String> {
        let suffix = |full: &str, prefix: &str| full.strip_prefix(prefix).map(str::to_owned).ok_or_else(|| "native stream differs from final message".to_string());
        let content = suffix(final_message["content"].as_str().unwrap_or_default(), &self.content)?;
        let mut calls = Vec::new();
        let final_calls = final_message["tool_calls"].as_array().cloned().unwrap_or_default();
        if self.calls.keys().any(|i| *i >= final_calls.len()) { return Err("final message lost streamed tool call".into()); }
        for (index, full) in final_calls.iter().enumerate() {
            let empty = json!({});
            let previous = self.calls.get(&index).unwrap_or(&empty);
            let id = suffix(full["id"].as_str().unwrap_or_default(), previous["id"].as_str().unwrap_or_default())?;
            let name = suffix(full["function"]["name"].as_str().unwrap_or_default(), previous["function"]["name"].as_str().unwrap_or_default())?;
            let args = suffix(full["function"]["arguments"].as_str().unwrap_or_default(), previous["function"]["arguments"].as_str().unwrap_or_default())?;
            if !id.is_empty() || !name.is_empty() || !args.is_empty() {
                let mut call = json!({"index":index,"function":{"arguments":args}});
                if !id.is_empty() { call["id"] = id.into(); call["type"] = "function".into(); }
                if !name.is_empty() { call["function"]["name"] = name.into(); }
                calls.push(call);
            }
        }
        let mut delta = json!({});
        if !content.is_empty() { delta["content"] = content.into(); }
        if !calls.is_empty() { delta["tool_calls"] = calls.into(); }
        Ok(delta)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_reconcile_without_duplicate_arguments() {
        let mut stream = NativeStream::default();
        stream.push(&json!({"tool_calls":[{"index":0,"id":"call_a","function":{"name":"write","arguments":"{\"text\":\""}}]})).unwrap();
        let final_message = json!({"tool_calls":[{"id":"call_a","type":"function","function":{"name":"write","arguments":"{\"text\":\"hello\"}"}}]});
        let remainder = stream.remainder(&final_message).unwrap();
        assert_eq!(remainder["tool_calls"][0]["function"]["arguments"], "hello\"}");
        stream.push(&remainder).unwrap();
        assert_eq!(stream.remainder(&final_message).unwrap(), json!({}));
        assert!(stream.remainder(&json!({"content":"different"})).is_err());
    }
    #[test]
    fn interleaved_calls_and_content_preserve_order() {
        let mut stream = NativeStream::default();
        stream.push(&json!({"content":"Hi", "tool_calls":[{"index":1,"id":"b","function":{"name":"two","arguments":"{"}},{"index":0,"id":"a","function":{"name":"one","arguments":"{}"}}]})).unwrap();
        stream.push(&json!({"tool_calls":[{"index":1,"function":{"arguments":"}"}}]})).unwrap();
        assert_eq!(stream.message()["tool_calls"][0]["id"], "a");
        assert_eq!(stream.message()["tool_calls"][1]["function"]["arguments"], "{}");
        assert!(stream.push(&json!({"tool_calls":[{"index":-1}]})).is_err());
    }
}
