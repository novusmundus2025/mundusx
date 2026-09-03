use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: SessionId,
    pub created_at_ms: u64,
    pub working_directory: String,
    pub title: Option<String>,
}

impl AgentSession {
    pub fn new(working_directory: impl Into<String>, title: Option<String>) -> Self {
        Self {
            id: SessionId::new(),
            created_at_ms: unix_time_ms(),
            working_directory: working_directory.into(),
            title,
        }
    }

    pub fn with_id(
        id: SessionId,
        working_directory: impl Into<String>,
        title: Option<String>,
    ) -> Self {
        Self {
            id,
            created_at_ms: unix_time_ms(),
            working_directory: working_directory.into(),
            title,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEventKind {
    SessionCreated {
        session: AgentSession,
    },
    UserMessageReceived {
        content: String,
    },
    ModelRequested {
        provider: String,
        model: String,
    },
    ModelResponded {
        content: Option<String>,
    },
    ToolProposed {
        call_id: String,
        tool: String,
        arguments: Value,
    },
    ApprovalRequested {
        call_id: String,
        reason: String,
    },
    ApprovalResolved {
        call_id: String,
        approved: bool,
    },
    ToolStarted {
        call_id: String,
    },
    ToolCompleted {
        call_id: String,
        result: Value,
        is_error: bool,
    },
    CheckpointSaved {
        summary: String,
    },
    ContextCompacted {
        original_chars: usize,
        retained_chars: usize,
    },
    SkillLoaded {
        name: String,
    },
    TaskDelegated {
        task_id: String,
        destination: String,
    },
    TaskCompleted {
        task_id: String,
    },
    FinalResponseProduced {
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub id: EventId,
    pub session_id: SessionId,
    pub sequence: u64,
    pub recorded_at_ms: u64,
    pub kind: AgentEventKind,
}

impl AgentEvent {
    pub fn new(session_id: SessionId, sequence: u64, kind: AgentEventKind) -> Self {
        Self {
            id: EventId::new(),
            session_id,
            sequence,
            recorded_at_ms: unix_time_ms(),
            kind,
        }
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
