//! Provider-neutral bounded model/tool loop for Coding Harness v1.
//!
//! The model can select only typed operations already authorized by the task.
//! This module never accepts a shell command or repository-defined executable.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoopBudgets {
    pub max_wall_time_ms: u64,
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
    pub max_repair_attempts: u32,
    pub max_output_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool_call_id: String,
    pub operation: String,
    pub idempotency_key: Option<String>,
    pub input: serde_json::Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ModelAction {
    Tool { request: ToolRequest },
    Finish { summary: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub operation: String,
    pub status: String,
    pub code: Option<String>,
    pub output: serde_json::Value,
    pub output_bytes: u64,
    pub progress_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopContext {
    pub turn: u32,
    pub repair_attempts: u32,
    pub remaining_tool_calls: u32,
    pub remaining_output_bytes: u64,
    pub last_result: Option<ToolResult>,
}

pub trait HarnessModel {
    fn next_action(&mut self, context: &LoopContext) -> Result<ModelAction, LoopFailure>;
}

pub trait HarnessTools {
    fn execute(&mut self, request: &ToolRequest) -> Result<ToolResult, LoopFailure>;
}

pub trait HarnessLoopObserver {
    fn model_turn(&mut self, _action: &ModelAction) -> Result<(), LoopFailure> {
        Ok(())
    }

    fn tool_call(
        &mut self,
        _request: &ToolRequest,
        _result: &ToolResult,
    ) -> Result<(), LoopFailure> {
        Ok(())
    }
}

struct NoopObserver;
impl HarnessLoopObserver for NoopObserver {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoopOutcome {
    pub status: String,
    pub code: Option<String>,
    pub summary: Option<String>,
    pub model_turns: u32,
    pub tool_calls: u32,
    pub repair_attempts: u32,
    pub output_bytes: u64,
    pub last_result: Option<ToolResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopFailure {
    pub code: &'static str,
    pub message: String,
}

impl LoopFailure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn run_bounded_loop<M, T, C>(
    model: &mut M,
    tools: &mut T,
    allowed_operations: &BTreeSet<String>,
    budgets: &LoopBudgets,
    cancelled: C,
) -> LoopOutcome
where
    M: HarnessModel,
    T: HarnessTools,
    C: FnMut() -> bool,
{
    run_bounded_loop_observed(
        model,
        tools,
        allowed_operations,
        budgets,
        &mut NoopObserver,
        cancelled,
    )
}

pub fn run_bounded_loop_observed<M, T, O, C>(
    model: &mut M,
    tools: &mut T,
    allowed_operations: &BTreeSet<String>,
    budgets: &LoopBudgets,
    observer: &mut O,
    mut cancelled: C,
) -> LoopOutcome
where
    M: HarnessModel,
    T: HarnessTools,
    O: HarnessLoopObserver,
    C: FnMut() -> bool,
{
    let started = Instant::now();
    let mut outcome = LoopOutcome {
        status: "running".to_string(),
        code: None,
        summary: None,
        model_turns: 0,
        tool_calls: 0,
        repair_attempts: 0,
        output_bytes: 0,
        last_result: None,
    };
    let mut last_progress = None::<String>;
    let mut repeated_progress = 0u32;

    loop {
        if cancelled() {
            return terminal(outcome, "cancelled", "HARNESS_CANCELLED");
        }
        if started.elapsed() >= Duration::from_millis(budgets.max_wall_time_ms)
            || outcome.model_turns >= budgets.max_model_turns
        {
            return terminal(outcome, "partial", "HARNESS_BUDGET_EXHAUSTED");
        }
        outcome.model_turns += 1;
        let context = LoopContext {
            turn: outcome.model_turns,
            repair_attempts: outcome.repair_attempts,
            remaining_tool_calls: budgets.max_tool_calls.saturating_sub(outcome.tool_calls),
            remaining_output_bytes: budgets
                .max_output_bytes
                .saturating_sub(outcome.output_bytes),
            last_result: outcome.last_result.clone(),
        };
        let action = match model.next_action(&context) {
            Ok(action) => action,
            Err(error) => return terminal(outcome, "failed", error.code),
        };
        if let Err(error) = observer.model_turn(&action) {
            return terminal(outcome, "failed", error.code);
        }
        match action {
            ModelAction::Finish { summary } => {
                let summary_bytes = summary.len() as u64;
                if summary.trim().is_empty()
                    || outcome.output_bytes.saturating_add(summary_bytes) > budgets.max_output_bytes
                {
                    return terminal(outcome, "partial", "HARNESS_BUDGET_EXHAUSTED");
                }
                outcome.output_bytes += summary_bytes;
                outcome.summary = Some(summary);
                outcome.status = "completed".to_string();
                return outcome;
            }
            ModelAction::Tool { request } => {
                if outcome.tool_calls >= budgets.max_tool_calls {
                    return terminal(outcome, "partial", "HARNESS_BUDGET_EXHAUSTED");
                }
                if !allowed_operations.contains(&request.operation) {
                    return terminal(outcome, "failed", "HARNESS_OPERATION_UNSUPPORTED");
                }
                if side_effecting(&request.operation)
                    && request
                        .idempotency_key
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty() || value.len() > 160)
                {
                    return terminal(outcome, "failed", "HARNESS_IDEMPOTENCY_REQUIRED");
                }
                outcome.tool_calls += 1;
                let result = match tools.execute(&request) {
                    Ok(result) => result,
                    Err(error) => return terminal(outcome, "failed", error.code),
                };
                if let Err(error) = observer.tool_call(&request, &result) {
                    return terminal(outcome, "failed", error.code);
                }
                if result.tool_call_id != request.tool_call_id
                    || result.operation != request.operation
                    || !is_sha256(&result.progress_sha256)
                    || outcome.output_bytes.saturating_add(result.output_bytes)
                        > budgets.max_output_bytes
                {
                    return terminal(outcome, "failed", "HARNESS_TOOL_RESULT_INVALID");
                }
                outcome.output_bytes += result.output_bytes;
                if last_progress.as_deref() == Some(&result.progress_sha256) {
                    repeated_progress += 1;
                } else {
                    last_progress = Some(result.progress_sha256.clone());
                    repeated_progress = 0;
                }
                if repeated_progress >= 3 {
                    outcome.last_result = Some(result);
                    return terminal(outcome, "partial", "HARNESS_NO_PROGRESS");
                }
                if request.operation == "validation.run" && result.status != "succeeded" {
                    outcome.repair_attempts += 1;
                    if outcome.repair_attempts > budgets.max_repair_attempts {
                        outcome.last_result = Some(result);
                        return terminal(outcome, "partial", "HARNESS_RETRY_EXHAUSTED");
                    }
                }
                outcome.last_result = Some(result);
            }
        }
    }
}

fn terminal(mut outcome: LoopOutcome, status: &str, code: &'static str) -> LoopOutcome {
    outcome.status = status.to_string();
    outcome.code = Some(code.to_string());
    outcome
}

fn side_effecting(operation: &str) -> bool {
    matches!(
        operation,
        "patch.apply" | "validation.run" | "artifact.publish"
    )
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn digest_progress(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FixtureModel(VecDeque<Result<ModelAction, LoopFailure>>);
    impl HarnessModel for FixtureModel {
        fn next_action(&mut self, _: &LoopContext) -> Result<ModelAction, LoopFailure> {
            self.0.pop_front().unwrap_or_else(|| {
                Err(LoopFailure::new(
                    "HARNESS_MODEL_OUTPUT_INVALID",
                    "fixture exhausted",
                ))
            })
        }
    }

    struct FixtureTools(VecDeque<ToolResult>);
    impl HarnessTools for FixtureTools {
        fn execute(&mut self, request: &ToolRequest) -> Result<ToolResult, LoopFailure> {
            let mut result = self.0.pop_front().ok_or_else(|| {
                LoopFailure::new("HARNESS_TOOL_RESULT_INVALID", "fixture exhausted")
            })?;
            result.tool_call_id = request.tool_call_id.clone();
            result.operation = request.operation.clone();
            Ok(result)
        }
    }

    fn tool(id: &str, operation: &str) -> ModelAction {
        ModelAction::Tool {
            request: ToolRequest {
                tool_call_id: id.to_string(),
                operation: operation.to_string(),
                idempotency_key: side_effecting(operation).then(|| format!("key-{id}")),
                input: serde_json::json!({}),
            },
        }
    }

    fn result(status: &str, progress: &str) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            operation: String::new(),
            status: status.to_string(),
            code: None,
            output: serde_json::json!({ "bounded": true }),
            output_bytes: 16,
            progress_sha256: progress.repeat(64),
        }
    }

    fn budgets() -> LoopBudgets {
        LoopBudgets {
            max_wall_time_ms: 5_000,
            max_model_turns: 12,
            max_tool_calls: 10,
            max_repair_attempts: 2,
            max_output_bytes: 4_096,
        }
    }

    fn allowed() -> BTreeSet<String> {
        ["file.read", "patch.apply", "validation.run"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn inspect_patch_fail_repair_validate_and_finish() {
        let mut model = FixtureModel(VecDeque::from(vec![
            Ok(tool("read", "file.read")),
            Ok(tool("patch-1", "patch.apply")),
            Ok(tool("test-1", "validation.run")),
            Ok(tool("patch-2", "patch.apply")),
            Ok(tool("test-2", "validation.run")),
            Ok(ModelAction::Finish {
                summary: "fixed and verified".to_string(),
            }),
        ]));
        let mut tools = FixtureTools(VecDeque::from(vec![
            result("succeeded", "1"),
            result("succeeded", "2"),
            result("failed", "3"),
            result("succeeded", "4"),
            result("succeeded", "5"),
        ]));
        let outcome = run_bounded_loop(&mut model, &mut tools, &allowed(), &budgets(), || false);
        assert_eq!(outcome.status, "completed");
        assert_eq!(outcome.repair_attempts, 1);
        assert_eq!(outcome.tool_calls, 5);
    }

    #[test]
    fn cancellation_timeout_unauthorized_and_no_progress_fail_closed() {
        let mut model = FixtureModel(VecDeque::new());
        let mut tools = FixtureTools(VecDeque::new());
        assert_eq!(
            run_bounded_loop(&mut model, &mut tools, &allowed(), &budgets(), || true).code,
            Some("HARNESS_CANCELLED".to_string())
        );

        let mut timeout_budget = budgets();
        timeout_budget.max_wall_time_ms = 0;
        assert_eq!(
            run_bounded_loop(
                &mut FixtureModel(VecDeque::new()),
                &mut FixtureTools(VecDeque::new()),
                &allowed(),
                &timeout_budget,
                || false,
            )
            .code,
            Some("HARNESS_BUDGET_EXHAUSTED".to_string())
        );

        let mut unauthorized = FixtureModel(VecDeque::from(vec![Ok(tool("x", "shell.exec"))]));
        assert_eq!(
            run_bounded_loop(
                &mut unauthorized,
                &mut FixtureTools(VecDeque::new()),
                &allowed(),
                &budgets(),
                || false,
            )
            .code,
            Some("HARNESS_OPERATION_UNSUPPORTED".to_string())
        );

        let actions = (0..4)
            .map(|index| Ok(tool(&format!("read-{index}"), "file.read")))
            .collect::<Vec<_>>();
        let results = (0..4).map(|_| result("succeeded", "a")).collect::<Vec<_>>();
        let outcome = run_bounded_loop(
            &mut FixtureModel(VecDeque::from(actions)),
            &mut FixtureTools(VecDeque::from(results)),
            &allowed(),
            &budgets(),
            || false,
        );
        assert_eq!(outcome.code, Some("HARNESS_NO_PROGRESS".to_string()));
    }
}
