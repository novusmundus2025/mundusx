use serde_json::Value;
use std::collections::HashSet;
use std::time::{Duration, Instant};

const MODEL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const TOOL_IDLE_TIMEOUT: Duration = Duration::from_secs(1800);

/// Transport heartbeats prove connectivity, not forward progress.
pub(super) struct ProgressWatchdog {
    last_progress: Instant,
    tools: HashSet<String>,
}

impl ProgressWatchdog {
    pub(super) fn new(now: Instant) -> Self {
        Self { last_progress: now, tools: HashSet::new() }
    }

    pub(super) fn observe(&mut self, event: &Value, now: Instant) {
        match event["type"].as_str().unwrap_or_default() {
            "tool_started" => {
                if let Some(id) = event["data"]["call_id"].as_str() {
                    self.tools.insert(id.to_string());
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
        let timeout = if self.tools.is_empty() { MODEL_IDLE_TIMEOUT } else { TOOL_IDLE_TIMEOUT };
        (now.duration_since(self.last_progress) >= timeout).then(|| {
            if self.tools.is_empty() {
                "Hermes model progress timed out after 5 minutes; project files and recovery checkpoint were preserved".to_string()
            } else {
                "Hermes tool progress stalled for 30 minutes; inspect the project before resuming".to_string()
            }
        })
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
            watch.observe(&json!({"type":"model_stream_progress"}), now + Duration::from_secs(seconds));
            assert!(watch.failure(now + Duration::from_secs(seconds + 299)).is_none());
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
        watch.observe(&json!({"type":"tool_completed","data":{"call_id":"a"}}), now);
        assert!(watch.failure(now + Duration::from_secs(600)).is_none());
        assert!(watch.failure(now + Duration::from_secs(1800)).unwrap().contains("tool progress"));
        watch.observe(&json!({"type":"tool_completed","data":{"call_id":"b"}}), now + Duration::from_secs(600));
        assert!(watch.failure(now + Duration::from_secs(900)).unwrap().contains("model progress"));
    }
}
