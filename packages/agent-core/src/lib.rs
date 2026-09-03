//! Provider-neutral building blocks for the local MundusX agent.
//!
//! This crate deliberately has no dependency on contributor infrastructure or
//! the hosted control plane. A local session must remain usable offline.

mod context;
mod event;
mod memory;
mod model;
mod runtime;
mod store;
mod tool;

pub use context::{compact_transcript, ContextWindow};
pub use event::{AgentEvent, AgentEventKind, AgentSession, EventId, SessionId};
pub use memory::{MemoryRecord, SqliteMemoryStore};
pub use model::{ModelError, ModelProvider, ModelRequest};
pub use runtime::{conversation_transcript, AgentRunError, AgentRunOutcome, AgentRunner};
pub use store::{AgentEventStore, SqliteEventStore, StoreError};
pub use tool::{
    ApprovalDecision, ApprovalProvider, ApprovalRequest, StaticApprovalProvider, Tool, ToolContext,
    ToolDefinition, ToolError, ToolRegistry,
};
