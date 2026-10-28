//! Typed HTTP planning; execution remains the client's registered terminal tool.
use serde_json::{json, Value};
pub const NAME: &str = "__mundusx_plan_http_request";

pub fn prepare(payload: &Value) -> Value {
    let Some(messages) = payload["messages"].as_array() else { return payload.clone(); };
    let policy = messages.iter().any(|m| m["name"] == "mundusx_native_http_policy");
    let windows = messages.iter().any(|m| m["content"].as_str().is_some_and(|s| s.contains("<environment_info>") && s.contains("The user's current OS is: Windows")));
    let start = messages.iter().rposition(|m| m["role"] == "user" && m["content"].as_str().is_some_and(|s|s.contains("<userRequest>"))).unwrap_or(0);
    let has_result = messages[start..].iter().any(|m|m["role"] == "tool");
    let choice = &payload["tool_choice"];
    let explicit = !choice.is_null() && !matches!(choice.as_str(), Some("auto" | "required"));
    let terminal = payload["tools"].as_array().and_then(|t|t.iter().find(|t|t["function"]["name"] == "run_in_terminal"));
    let compatible = terminal.is_some_and(|t|t["function"]["parameters"]["required"].as_array().is_some_and(|r|r.iter().all(|v|matches!(v.as_str(),Some("command" | "goal" | "explanation" | "mode")))) && t["function"]["parameters"]["properties"]["mode"]["enum"].as_array().is_some_and(|v|v.iter().any(|v|v=="sync")));
    if !policy || !windows || has_result || explicit || !compatible { return payload.clone(); }
    let mut result = payload.clone();
    result["tools"] = json!([{"type":"function","function":{"name":NAME,"description":"Plan the user's HTTP request as structured data. A client-side terminal adapter executes it using curl.exe. Do not write shell commands or claim execution. Preserve the actual URL, method, headers and JSON fields from the task. Do not replay successful mutations.","parameters":{"type":"object","properties":{"method":{"type":"string","enum":["GET","POST","PUT","PATCH","DELETE","HEAD","OPTIONS"]},"url":{"type":"string"},"headers":{"type":"object","additionalProperties":{"type":"string"}},"json_body":{"type":"object","description":"Actual JSON request fields and values, without shell escaping."}},"required":["method","url"],"additionalProperties":false}}}]);
    result["tool_choice"] = json!("required");
    result
}

fn quote(text: &str) -> String { format!("'{}'",text.replace('\'',"''")) }

pub fn compile(payload:&Value, message:&Value)->Result<Value,String> {
    let Some(calls)=message["tool_calls"].as_array() else{return Ok(message.clone());};
    if !calls.iter().any(|c|c["function"]["name"]==NAME) {return Ok(message.clone());}
    if prepare(payload)["tools"][0]["function"]["name"]!=NAME || calls.len()!=1 {return Err("native tool schema precondition failed: private HTTP planner is unavailable for this turn".into());}
    let call=&calls[0];
    let args:Value=serde_json::from_str(call["function"]["arguments"].as_str().unwrap_or("")).map_err(|_|"native tool schema precondition failed: HTTP plan arguments must be JSON")?;
    let method=args["method"].as_str().filter(|m|matches!(*m,"GET"|"POST"|"PUT"|"PATCH"|"DELETE"|"HEAD"|"OPTIONS")).ok_or("native tool schema precondition failed: HTTP plan method is invalid")?;
    let address=args["url"].as_str().ok_or("native tool schema precondition failed: HTTP plan URL is required")?;
    let parsed=url::Url::parse(address).map_err(|_|"native tool schema precondition failed: HTTP plan URL is invalid")?;
    if !matches!(parsed.scheme(),"http"|"https") || address.chars().any(char::is_control) {return Err("native tool schema precondition failed: HTTP plan requires an HTTP URL".into());}
    let mut headers=Vec::new();let mut content_type=false;
    if let Some(value)=args.get("headers") {
        let values=value.as_object().ok_or("native tool schema precondition failed: HTTP headers must be an object")?;
        for (key,value) in values {
            let value=value.as_str().ok_or("native tool schema precondition failed: HTTP header values must be strings")?;
            if key.is_empty() || key.contains(':') || key.chars().chain(value.chars()).any(char::is_control) {return Err("native tool schema precondition failed: HTTP header is invalid".into());}
            content_type |= key.eq_ignore_ascii_case("content-type");headers.push(format!(" -H {}",quote(&format!("{key}: {value}"))));
        }
    }
    let mut prelude=String::new();let mut body_arg=String::new();let mut cleanup=String::new();
    if let Some(body)=args.get("json_body") {
        if !body.is_object(){return Err("native tool schema precondition failed: HTTP json_body must be an object".into());}
        let bytes=serde_json::to_vec(body).map_err(|_|"native tool schema precondition failed: HTTP body is invalid")?;
        if bytes.len()>16384 {return Err("native tool schema precondition failed: HTTP body exceeds the adapter limit".into());}
        let literal=bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
        prelude=format!("$mundusxPayload = [IO.Path]::GetTempFileName(); [IO.File]::WriteAllBytes($mundusxPayload,[byte[]]@({literal})); try {{ ");
        body_arg=" --data-binary ('@' + $mundusxPayload)".into();cleanup=" } finally { Remove-Item -LiteralPath $mundusxPayload -ErrorAction SilentlyContinue }".into();
        if !content_type {headers.push(" -H 'Content-Type: application/json'".into());}
    }
    let command=format!("$ErrorActionPreference = 'Stop'; {prelude}curl.exe -sS --fail-with-body --request {method} {}{}{body_arg}; if ($LASTEXITCODE -ne 0) {{ throw ('curl exited with code ' + $LASTEXITCODE) }}{cleanup}",quote(address),headers.join(""));
    let arguments=json!({"command":command,"mode":"sync","goal":format!("Send requested {method} request"),"explanation":"Execute the structured HTTP request and return the actual response."});
    let mut out=message.clone();out["content"]=Value::Null;out["tool_calls"]=json!([{"id":call["id"],"type":"function","function":{"name":"run_in_terminal","arguments":arguments.to_string()}}]);Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->Value {json!({"messages":[{"role":"system","name":"mundusx_native_http_policy","content":"HTTP task"},{"role":"user","content":"<environment_info>The user's current OS is: Windows</environment_info><userRequest>create a record using curl</userRequest>"}],"tools":[{"function":{"name":"run_in_terminal","parameters":{"required":["command","goal","explanation","mode"],"properties":{"mode":{"enum":["sync","async"]}}}}}]})}
    fn call(args:Value)->Value {json!({"tool_calls":[{"id":"test-call","function":{"name":NAME,"arguments":args.to_string()}}]})}
    #[test] fn planner_is_scoped_and_does_not_replace_host_results_or_explicit_choices() {
        let p=fixture();assert_eq!(prepare(&p)["tools"][0]["function"]["name"],NAME);
        let mut result=p.clone();result["messages"].as_array_mut().unwrap().push(json!({"role":"tool","content":"HTTP 201 Created"}));assert_eq!(prepare(&result),result);
        let mut explicit=p.clone();explicit["tool_choice"]=json!("none");assert_eq!(prepare(&explicit),explicit);
        let mut other=p.clone();other["messages"][0]["name"]=json!("coding_policy");assert_eq!(prepare(&other),other);
    }
    #[test] fn compilation_quotes_data_and_never_executes_or_leaks_private_tool_names() {
        let body=json!({"name":"O'Brien $() 中文","age":20});let args=json!({"method":"POST","url":"http://localhost:3000/api/students","json_body":body});
        let result=compile(&fixture(),&call(args)).unwrap();assert_eq!(result["tool_calls"][0]["function"]["name"],"run_in_terminal");assert_eq!(result["tool_calls"][0]["id"],"test-call");
        let args:Value=serde_json::from_str(result["tool_calls"][0]["function"]["arguments"].as_str().unwrap()).unwrap();let command=args["command"].as_str().unwrap();assert!(command.contains("[IO.File]::WriteAllBytes"));assert!(command.contains("finally"));assert!(!command.contains("O'Brien"));assert!(!command.contains(NAME));
    }
    #[test] fn invalid_plans_fail_before_host_call() {
        for args in [json!({"method":"BAD","url":"http://localhost"}),json!({"method":"POST","url":"file:///etc/passwd"}),json!({"method":"POST","url":"http://localhost","headers":{"X-Test":"bad\r\nheader"}}),json!({"method":"POST","url":"http://localhost","json_body":"shell text"})] {assert!(compile(&fixture(),&call(args)).is_err());}
    }
}
