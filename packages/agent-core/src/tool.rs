use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub struct ToolContext {
    workspace: PathBuf,
}

impl ToolContext {
    pub fn new(workspace: impl Into<PathBuf>) -> Result<Self, ToolError> {
        let workspace = workspace.into();
        let workspace = workspace.canonicalize().map_err(|error| {
            ToolError::new(format!(
                "could not resolve workspace {}: {error}",
                workspace.display()
            ))
        })?;
        if !workspace.is_dir() {
            return Err(ToolError::new("workspace must be a directory"));
        }
        Ok(Self { workspace })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn resolve_existing(&self, relative: &str) -> Result<PathBuf, ToolError> {
        let candidate = self.workspace.join(relative);
        let resolved = candidate.canonicalize().map_err(|error| {
            ToolError::new(format!(
                "could not resolve {}: {error}",
                candidate.display()
            ))
        })?;
        if !resolved.starts_with(&self.workspace) {
            return Err(ToolError::new("path escapes the workspace"));
        }
        Ok(resolved)
    }
}

#[derive(Clone, Debug)]
pub struct ToolError {
    pub message: String,
}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ToolError {}

pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    fn execute(&self, arguments: &Value, context: &ToolContext) -> Result<Value, ToolError>;
}

#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub reason: String,
}

pub trait ApprovalProvider {
    fn decide(&mut self, request: &ApprovalRequest) -> ApprovalDecision;
}

pub struct StaticApprovalProvider {
    allow_mutations: bool,
}

impl StaticApprovalProvider {
    pub fn read_only() -> Self {
        Self {
            allow_mutations: false,
        }
    }

    pub fn allow_mutations() -> Self {
        Self {
            allow_mutations: true,
        }
    }
}

impl ApprovalProvider for StaticApprovalProvider {
    fn decide(&mut self, request: &ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision {
            approved: self.allow_mutations,
            reason: if self.allow_mutations {
                format!("{} was explicitly allowed for this request", request.tool)
            } else {
                format!("{} requires explicit mutation approval", request.tool)
            },
        }
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> Result<(), ToolError> {
        let definition = tool.definition();
        if definition.name.trim().is_empty() {
            return Err(ToolError::new("tool name cannot be empty"));
        }
        if self
            .tools
            .insert(definition.name.clone(), Box::new(tool))
            .is_some()
        {
            return Err(ToolError::new(format!(
                "tool {} is already registered",
                definition.name
            )));
        }
        Ok(())
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }

    pub fn definition(&self, name: &str) -> Option<ToolDefinition> {
        self.tools.get(name).map(|tool| tool.definition())
    }

    pub fn execute(
        &self,
        name: &str,
        arguments: &Value,
        context: &ToolContext,
    ) -> Result<Value, ToolError> {
        self.tools
            .get(name)
            .ok_or_else(|| ToolError::new(format!("unknown tool: {name}")))?
            .execute(arguments, context)
    }
}
