use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const MODEL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const TOOL_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const TERMINAL_IDLE_TIMEOUT: Duration = Duration::from_secs(900);
const BROWSER_IDLE_TIMEOUT: Duration = Duration::from_secs(180);
const CODE_EXECUTION_IDLE_TIMEOUT: Duration = Duration::from_secs(360);

fn tool_idle_timeout(name: &str) -> Duration {
    match name {
        name if name.starts_with("browser_") => BROWSER_IDLE_TIMEOUT,
        "execute_code" => CODE_EXECUTION_IDLE_TIMEOUT,
        "terminal" | "execute" | "shell" => TERMINAL_IDLE_TIMEOUT,
        _ => TOOL_IDLE_TIMEOUT,
    }
}

/// Transport heartbeats prove connectivity, not forward progress.
pub(super) struct ProgressWatchdog {
    last_progress: Instant,
    tools: HashMap<String, String>,
}

impl ProgressWatchdog {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            last_progress: now,
            tools: HashMap::new(),
        }
    }

    pub(super) fn observe(&mut self, event: &Value, now: Instant) {
        match event["type"].as_str().unwrap_or_default() {
            "tool_started" => {
                if let Some(id) = event["data"]["call_id"].as_str() {
                    self.tools.insert(
                        id.to_string(),
                        event["data"]["name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    );
                }
            }
            "tool_completed" => {
                if let Some(id) = event["data"]["call_id"].as_str() {
                    self.tools.remove(id);
                }
            }
            "model_requested" | "model_response_received" | "model_stream_progress" => {}
            _ => return,
        }
        self.last_progress = now;
    }

    pub(super) fn failure(&self, now: Instant) -> Option<String> {
        let active_tool = self
            .tools
            .values()
            .min_by_key(|name| tool_idle_timeout(name))
            .map(String::as_str);
        let timeout = active_tool
            .map(tool_idle_timeout)
            .unwrap_or(MODEL_IDLE_TIMEOUT);
        (now.duration_since(self.last_progress) >= timeout).then(|| {
            if self.tools.is_empty() {
                "Hermes model progress timed out after 5 minutes; project files and recovery checkpoint were preserved".to_string()
            } else {
                let name = active_tool.filter(|name| !name.is_empty()).unwrap_or("tool");
                format!(
                    "Hermes {} progress stalled after {} minutes; project files and recovery checkpoint were preserved",
                    name,
                    timeout.as_secs() / 60,
                )
            }
        })
    }

    pub(super) fn interrupted_tool_events(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|(call_id, name)| {
                serde_json::json!({
                    "type": "tool_completed",
                    "data": {
                        "call_id": call_id,
                        "name": name,
                        "success": false,
                        "timed_out": true,
                        "activity": if name.starts_with("browser_") {
                            "browser_acceptance"
                        } else if name == "execute_code" {
                            "code"
                        } else {
                            "tool"
                        }
                    }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn heartbeats_do_not_hide_a_stalled_model() {
        let now = Instant::now();
        let mut watch = ProgressWatchdog::new(now);
        watch.observe(&json!({"type":"heartbeat"}), now + Duration::from_secs(299));
        assert!(watch.failure(now + Duration::from_secs(300)).is_some());
    }

    #[test]
    fn actual_model_output_extends_the_deadline() {
        let now = Instant::now();
        let mut watch = ProgressWatchdog::new(now);
        for seconds in [240, 480, 720] {
            watch.observe(
                &json!({"type":"model_stream_progress"}),
                now + Duration::from_secs(seconds),
            );
            assert!(watch
                .failure(now + Duration::from_secs(seconds + 299))
                .is_none());
        }
        assert!(watch.failure(now + Duration::from_secs(1020)).is_some());
    }

    #[test]
    fn parallel_tools_get_a_separate_deadline_then_return_to_model_wait() {
        let now = Instant::now();
        let mut watch = ProgressWatchdog::new(now);
        for id in ["a", "b"] {
            watch.observe(&json!({"type":"tool_started","data":{"call_id":id}}), now);
        }
        watch.observe(
            &json!({"type":"tool_completed","data":{"call_id":"a"}}),
            now,
        );
        assert!(watch.failure(now + Duration::from_secs(599)).is_none());
        assert!(watch
            .failure(now + Duration::from_secs(600))
            .unwrap()
            .contains("tool progress"));
        watch.observe(
            &json!({"type":"tool_completed","data":{"call_id":"b"}}),
            now + Duration::from_secs(600),
        );
        assert!(watch
            .failure(now + Duration::from_secs(900))
            .unwrap()
            .contains("model progress"));
    }

    #[test]
    fn browser_stalls_quickly_but_code_and_terminal_get_their_own_deadlines() {
        let now = Instant::now();
        let mut browser = ProgressWatchdog::new(now);
        browser.observe(
            &json!({"type":"tool_started","data":{"call_id":"browser","name":"browser_exec"}}),
            now,
        );
        assert!(browser.failure(now + Duration::from_secs(179)).is_none());
        assert!(browser
            .failure(now + Duration::from_secs(180))
            .unwrap()
            .contains("browser_exec"));

        let mut code = ProgressWatchdog::new(now);
        code.observe(
            &json!({"type":"tool_started","data":{"call_id":"code","name":"execute_code"}}),
            now,
        );
        assert!(code.failure(now + Duration::from_secs(359)).is_none());
        assert!(code
            .failure(now + Duration::from_secs(360))
            .unwrap()
            .contains("execute_code"));
        let interrupted = code.interrupted_tool_events();
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0]["data"]["success"], false);
        assert_eq!(interrupted[0]["data"]["activity"], "code");

        let mut terminal = ProgressWatchdog::new(now);
        terminal.observe(
            &json!({"type":"tool_started","data":{"call_id":"terminal","name":"terminal"}}),
            now,
        );
        assert!(terminal.failure(now + Duration::from_secs(899)).is_none());
        assert!(terminal.failure(now + Duration::from_secs(900)).is_some());
    }
}
