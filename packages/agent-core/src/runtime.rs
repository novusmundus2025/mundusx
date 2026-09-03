use crate::{
    compact_transcript, AgentEvent, AgentEventKind, AgentEventStore, ApprovalProvider,
    ApprovalRequest, ModelProvider, ModelRequest, SessionId, StaticApprovalProvider, StoreError,
    ToolContext, ToolRegistry,
};
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_MAX_TURNS: u32 = 12;

pub fn conversation_transcript(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            AgentEventKind::UserMessageReceived { content } => Some(format!("USER:\n{content}")),
            AgentEventKind::FinalResponseProduced { content } => {
                Some(format!("ASSISTANT:\n{content}"))
            }
            AgentEventKind::CheckpointSaved { summary } => Some(format!("CHECKPOINT:\n{summary}")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ModelAction {
    ToolCall {
        call_id: String,
        tool: String,
        #[serde(default)]
        arguments: Value,
    },
    Final {
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRunOutcome {
    pub content: String,
    pub turns: u32,
    pub tool_calls: u32,
    pub next_sequence: u64,
}

#[derive(Debug)]
pub enum AgentRunError {
    Model(String),
    Protocol(String),
    Store(StoreError),
    TurnLimit(u32),
}

impl std::fmt::Display for AgentRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model(message) => write!(formatter, "model failed: {message}"),
            Self::Protocol(message) => write!(formatter, "model protocol failed: {message}"),
            Self::Store(error) => error.fmt(formatter),
            Self::TurnLimit(limit) => write!(formatter, "agent reached its {limit}-turn limit"),
        }
    }
}

impl std::error::Error for AgentRunError {}

impl From<StoreError> for AgentRunError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

pub struct AgentRunner<'a> {
    provider: &'a mut dyn ModelProvider,
    tools: &'a ToolRegistry,
    max_turns: u32,
    max_tokens: u32,
    temperature: f32,
    max_tool_calls: u32,
    approval: Option<&'a mut dyn ApprovalProvider>,
    max_context_chars: usize,
    additional_instructions: Vec<String>,
}

impl<'a> AgentRunner<'a> {
    pub fn new(provider: &'a mut dyn ModelProvider, tools: &'a ToolRegistry) -> Self {
        Self {
            provider,
            tools,
            max_turns: DEFAULT_MAX_TURNS,
            max_tokens: 1024,
            temperature: 0.1,
            max_tool_calls: 32,
            approval: None,
            max_context_chars: 64 * 1024,
            additional_instructions: Vec::new(),
        }
    }

    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.max_turns = max_turns.max(1);
        self
    }

    pub fn with_generation(mut self, max_tokens: Option<u32>, temperature: Option<f32>) -> Self {
        self.max_tokens = max_tokens.unwrap_or(1024).clamp(64, 16_384);
        self.temperature = temperature.unwrap_or(0.1).clamp(0.0, 2.0);
        self
    }

    pub fn with_max_tool_calls(mut self, max_tool_calls: u32) -> Self {
        self.max_tool_calls = max_tool_calls.max(1);
        self
    }

    pub fn with_approval_provider(mut self, approval: &'a mut dyn ApprovalProvider) -> Self {
        self.approval = Some(approval);
        self
    }

    pub fn with_context_limit(mut self, max_context_chars: usize) -> Self {
        self.max_context_chars = max_context_chars.max(1024);
        self
    }

    pub fn with_instructions(mut self, instructions: Vec<String>) -> Self {
        self.additional_instructions = instructions;
        self
    }

    pub fn run(
        &mut self,
        store: &mut dyn AgentEventStore,
        session_id: SessionId,
        mut sequence: u64,
        user_input: &str,
        prior_transcript: &str,
        context: &ToolContext,
    ) -> Result<AgentRunOutcome, AgentRunError> {
        let definitions = serde_json::to_string_pretty(&self.tools.definitions())
            .map_err(|error| AgentRunError::Protocol(error.to_string()))?;
        let mut system_prompt = format!(
            "You are the local MundusX agent. Work only through the tools listed below. \
Return exactly one JSON object and no markdown. To use a tool return \
{{\"type\":\"tool_call\",\"call_id\":\"unique-id\",\"tool\":\"tool.name\",\"arguments\":{{}}}}. \
When the task is complete return {{\"type\":\"final\",\"content\":\"answer\"}}.\n\nTools:\n{definitions}"
        );
        if !self.additional_instructions.is_empty() {
            system_prompt.push_str("\n\nSelected skills:\n");
            system_prompt.push_str(&self.additional_instructions.join("\n\n"));
        }
        let mut transcript = prior_transcript.to_string();
        if !transcript.is_empty() {
            transcript.push_str("\n\n");
        }
        transcript.push_str("USER:\n");
        transcript.push_str(user_input);
        let mut tool_calls = 0;

        for turn in 1..=self.max_turns {
            let context_window = compact_transcript(&transcript, self.max_context_chars);
            if context_window.compacted {
                store.append(&AgentEvent::new(
                    session_id.clone(),
                    sequence,
                    AgentEventKind::ContextCompacted {
                        original_chars: context_window.original_chars,
                        retained_chars: context_window.text.len(),
                    },
                ))?;
                sequence += 1;
                transcript = context_window.text;
            }
            store.append(&AgentEvent::new(
                session_id.clone(),
                sequence,
                AgentEventKind::ModelRequested {
                    provider: self.provider.provider_name().to_string(),
                    model: self.provider.model_name().to_string(),
                },
            ))?;
            sequence += 1;
            let raw = self
                .provider
                .complete(&ModelRequest {
                    system_prompt: system_prompt.clone(),
                    transcript: transcript.clone(),
                    max_tokens: Some(self.max_tokens),
                    temperature: Some(self.temperature),
                })
                .map_err(|error| AgentRunError::Model(error.to_string()))?;
            store.append(&AgentEvent::new(
                session_id.clone(),
                sequence,
                AgentEventKind::ModelResponded {
                    content: Some(raw.clone()),
                },
            ))?;
            sequence += 1;
            let action = parse_action(&raw)?;
            match action {
                ModelAction::Final { content } => {
                    store.append(&AgentEvent::new(
                        session_id,
                        sequence,
                        AgentEventKind::FinalResponseProduced {
                            content: content.clone(),
                        },
                    ))?;
                    return Ok(AgentRunOutcome {
                        content,
                        turns: turn,
                        tool_calls,
                        next_sequence: sequence + 1,
                    });
                }
                ModelAction::ToolCall {
                    call_id,
                    tool,
                    arguments,
                } => {
                    tool_calls += 1;
                    if tool_calls > self.max_tool_calls {
                        return Err(AgentRunError::Protocol(format!(
                            "agent exceeded its {} tool-call limit",
                            self.max_tool_calls
                        )));
                    }
                    store.append(&AgentEvent::new(
                        session_id.clone(),
                        sequence,
                        AgentEventKind::ToolProposed {
                            call_id: call_id.clone(),
                            tool: tool.clone(),
                            arguments: arguments.clone(),
                        },
                    ))?;
                    sequence += 1;
                    if self
                        .tools
                        .definition(&tool)
                        .is_some_and(|definition| !definition.read_only)
                    {
                        let reason = format!("{tool} can change local state or execute a process");
                        let approval_request = ApprovalRequest {
                            call_id: call_id.clone(),
                            tool: tool.clone(),
                            arguments: arguments.clone(),
                            reason: reason.clone(),
                        };
                        store.append(&AgentEvent::new(
                            session_id.clone(),
                            sequence,
                            AgentEventKind::ApprovalRequested {
                                call_id: call_id.clone(),
                                reason,
                            },
                        ))?;
                        sequence += 1;
                        let decision = match self.approval.as_deref_mut() {
                            Some(provider) => provider.decide(&approval_request),
                            None => StaticApprovalProvider::read_only().decide(&approval_request),
                        };
                        store.append(&AgentEvent::new(
                            session_id.clone(),
                            sequence,
                            AgentEventKind::ApprovalResolved {
                                call_id: call_id.clone(),
                                approved: decision.approved,
                            },
                        ))?;
                        sequence += 1;
                        if !decision.approved {
                            let payload = serde_json::json!({"error": decision.reason});
                            store.append(&AgentEvent::new(
                                session_id.clone(),
                                sequence,
                                AgentEventKind::ToolCompleted {
                                    call_id: call_id.clone(),
                                    result: payload.clone(),
                                    is_error: true,
                                },
                            ))?;
                            sequence += 1;
                            transcript.push_str(&format!(
                                "\n\nASSISTANT_ACTION:\n{raw}\n\nTOOL_RESULT {call_id}:\n{}",
                                serde_json::to_string(&payload).map_err(|error| {
                                    AgentRunError::Protocol(error.to_string())
                                })?
                            ));
                            continue;
                        }
                    }
                    store.append(&AgentEvent::new(
                        session_id.clone(),
                        sequence,
                        AgentEventKind::ToolStarted {
                            call_id: call_id.clone(),
                        },
                    ))?;
                    sequence += 1;
                    let result = self.tools.execute(&tool, &arguments, context);
                    let (payload, is_error) = match result {
                        Ok(value) => (value, false),
                        Err(error) => (serde_json::json!({"error": error.to_string()}), true),
                    };
                    store.append(&AgentEvent::new(
                        session_id.clone(),
                        sequence,
                        AgentEventKind::ToolCompleted {
                            call_id: call_id.clone(),
                            result: payload.clone(),
                            is_error,
                        },
                    ))?;
                    sequence += 1;
                    transcript.push_str(&format!(
                        "\n\nASSISTANT_ACTION:\n{raw}\n\nTOOL_RESULT {call_id}:\n{}",
                        serde_json::to_string(&payload)
                            .map_err(|error| AgentRunError::Protocol(error.to_string()))?
                    ));
                }
            }
        }
        Err(AgentRunError::TurnLimit(self.max_turns))
    }
}

fn parse_action(raw: &str) -> Result<ModelAction, AgentRunError> {
    let trimmed = raw.trim();
    let json = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    serde_json::from_str(json)
        .map_err(|error| AgentRunError::Protocol(format!("{error}; response was {trimmed}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentSession, ModelError, SqliteEventStore, Tool, ToolDefinition, ToolError};
    use serde_json::json;
    use std::collections::VecDeque;

    struct ScriptedProvider {
        replies: VecDeque<String>,
    }

    impl ModelProvider for ScriptedProvider {
        fn provider_name(&self) -> &str {
            "test"
        }
        fn model_name(&self) -> &str {
            "scripted"
        }
        fn complete(&mut self, _: &ModelRequest) -> Result<String, ModelError> {
            self.replies
                .pop_front()
                .ok_or_else(|| ModelError::new("no reply"))
        }
    }

    struct EchoTool;
    impl Tool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".to_string(),
                description: "Echo input".to_string(),
                input_schema: json!({"type":"object"}),
                read_only: true,
            }
        }
        fn execute(&self, arguments: &Value, _: &ToolContext) -> Result<Value, ToolError> {
            Ok(arguments.clone())
        }
    }

    #[test]
    fn loops_through_a_tool_call_before_returning_final_output() {
        let workspace = std::env::current_dir().expect("cwd");
        let context = ToolContext::new(workspace).expect("context");
        let mut store = SqliteEventStore::in_memory().expect("store");
        let session = AgentSession::new(".", None);
        store.create_session(&session).expect("session");
        let mut tools = ToolRegistry::default();
        tools.register(EchoTool).expect("tool");
        let mut provider = ScriptedProvider {
            replies: VecDeque::from([
                r#"{"type":"tool_call","call_id":"one","tool":"echo","arguments":{"value":1}}"#
                    .to_string(),
                r#"{"type":"final","content":"done"}"#.to_string(),
            ]),
        };
        let outcome = AgentRunner::new(&mut provider, &tools)
            .run(&mut store, session.id.clone(), 1, "do it", "", &context)
            .expect("run");
        assert_eq!(outcome.content, "done");
        assert_eq!(outcome.tool_calls, 1);
        assert_eq!(outcome.turns, 2);
        assert_eq!(store.events(&session.id).expect("events").len(), 8);
    }

    #[test]
    fn reconstructs_only_durable_conversation_and_checkpoint_context() {
        let session = SessionId::new();
        let events = vec![
            AgentEvent::new(
                session.clone(),
                1,
                AgentEventKind::UserMessageReceived {
                    content: "first".to_string(),
                },
            ),
            AgentEvent::new(
                session.clone(),
                2,
                AgentEventKind::ModelResponded {
                    content: Some("internal protocol".to_string()),
                },
            ),
            AgentEvent::new(
                session,
                3,
                AgentEventKind::FinalResponseProduced {
                    content: "answer".to_string(),
                },
            ),
        ];
        assert_eq!(
            conversation_transcript(&events),
            "USER:\nfirst\n\nASSISTANT:\nanswer"
        );
    }
}
