use mundusx_agent_core::{
    SqliteMemoryStore, Tool, ToolContext, ToolDefinition, ToolError, ToolRegistry,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug)]
pub struct ValidationProfile {
    pub program: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ControlPlaneDelegation {
    pub base_url: String,
    pub bearer_token: Option<String>,
}

pub fn register_control_plane_delegation(
    registry: &mut ToolRegistry,
    config: ControlPlaneDelegation,
) -> Result<(), ToolError> {
    registry.register(TaskDelegate { config })
}

pub fn read_only_registry() -> Result<ToolRegistry, ToolError> {
    let mut registry = ToolRegistry::default();
    registry.register(FileRead)?;
    registry.register(FileSearch)?;
    registry.register(RepositoryStatus)?;
    registry.register(RepositoryDiff)?;
    Ok(registry)
}

pub fn coding_registry(
    validation_profiles: std::collections::BTreeMap<String, ValidationProfile>,
) -> Result<ToolRegistry, ToolError> {
    let mut registry = read_only_registry()?;
    registry.register(PatchApply)?;
    registry.register(ValidationRun {
        profiles: validation_profiles,
    })?;
    Ok(registry)
}

pub fn full_registry(
    validation_profiles: std::collections::BTreeMap<String, ValidationProfile>,
    memory_path: PathBuf,
) -> Result<ToolRegistry, ToolError> {
    let mut registry = coding_registry(validation_profiles)?;
    registry.register(MemorySearch {
        path: memory_path.clone(),
    })?;
    registry.register(MemorySave { path: memory_path })?;
    Ok(registry)
}

struct FileRead;
impl Tool for FileRead {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file.read".to_string(),
            description: "Read a UTF-8 text file inside the workspace".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
                "additionalProperties": false
            }),
            read_only: true,
        }
    }

    fn execute(&self, arguments: &Value, context: &ToolContext) -> Result<Value, ToolError> {
        let path = required_string(arguments, "path")?;
        let resolved = context.resolve_existing(path)?;
        if !resolved.is_file() {
            return Err(ToolError::new("path is not a file"));
        }
        let metadata =
            fs::metadata(&resolved).map_err(|error| ToolError::new(error.to_string()))?;
        if metadata.len() > 1024 * 1024 {
            return Err(ToolError::new("file exceeds the 1 MiB read limit"));
        }
        let content =
            fs::read_to_string(&resolved).map_err(|error| ToolError::new(error.to_string()))?;
        Ok(json!({"path": path, "content": content}))
    }
}

struct FileSearch;
impl Tool for FileSearch {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file.search".to_string(),
            description: "Search workspace text using ripgrep".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "path": {"type": "string", "default": "."}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            read_only: true,
        }
    }

    fn execute(&self, arguments: &Value, context: &ToolContext) -> Result<Value, ToolError> {
        let query = required_string(arguments, "query")?;
        let relative = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let target = context.resolve_existing(relative)?;
        let output = Command::new("rg")
            .args([
                "--line-number",
                "--color",
                "never",
                "--max-count",
                "100",
                "--",
            ])
            .arg(query)
            .arg(target)
            .current_dir(context.workspace())
            .output()
            .map_err(|error| ToolError::new(format!("could not run rg: {error}")))?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(ToolError::new(
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        Ok(json!({
            "matches": bounded_output(&output.stdout),
            "truncated": output.stdout.len() > 256 * 1024
        }))
    }
}

struct RepositoryStatus;
impl Tool for RepositoryStatus {
    fn definition(&self) -> ToolDefinition {
        git_definition("repository.status", "Show the workspace Git status")
    }
    fn execute(&self, _: &Value, context: &ToolContext) -> Result<Value, ToolError> {
        git_output(context, &["status", "--short", "--branch"])
    }
}

struct RepositoryDiff;
impl Tool for RepositoryDiff {
    fn definition(&self) -> ToolDefinition {
        git_definition(
            "repository.diff",
            "Show unstaged and staged workspace changes",
        )
    }
    fn execute(&self, _: &Value, context: &ToolContext) -> Result<Value, ToolError> {
        let unstaged = git_text(context, &["diff", "--"])?;
        let staged = git_text(context, &["diff", "--cached", "--"])?;
        Ok(json!({"unstaged": unstaged, "staged": staged}))
    }
}

struct PatchApply;
impl Tool for PatchApply {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "patch.apply".to_string(),
            description: "Replace one exact text occurrence in an existing workspace file"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "expected": {"type": "string"},
                    "replacement": {"type": "string"}
                },
                "required": ["path", "expected", "replacement"],
                "additionalProperties": false
            }),
            read_only: false,
        }
    }

    fn execute(&self, arguments: &Value, context: &ToolContext) -> Result<Value, ToolError> {
        let relative = required_string(arguments, "path")?;
        let expected = required_string(arguments, "expected")?;
        let replacement = arguments
            .get("replacement")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::new("replacement must be a string"))?;
        let path = context.resolve_existing(relative)?;
        if !path.is_file() {
            return Err(ToolError::new("path is not a file"));
        }
        let mut content = fs::read_to_string(&path)
            .map_err(|error| ToolError::new(format!("could not read {relative}: {error}")))?;
        if content.len() > 1024 * 1024 {
            return Err(ToolError::new("file exceeds the 1 MiB patch limit"));
        }
        let occurrences = content.matches(expected).count();
        if occurrences != 1 {
            return Err(ToolError::new(format!(
                "expected text must occur exactly once; found {occurrences} occurrences"
            )));
        }
        content = content.replacen(expected, replacement, 1);
        fs::write(&path, content)
            .map_err(|error| ToolError::new(format!("could not write {relative}: {error}")))?;
        Ok(json!({"path": relative, "changed": true}))
    }
}

struct ValidationRun {
    profiles: std::collections::BTreeMap<String, ValidationProfile>,
}

struct MemorySearch {
    path: PathBuf,
}

impl Tool for MemorySearch {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory.search".to_string(),
            description: "Search durable local memories for relevant facts".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            read_only: true,
        }
    }

    fn execute(&self, arguments: &Value, _: &ToolContext) -> Result<Value, ToolError> {
        let query = required_string(arguments, "query")?;
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
        let store = SqliteMemoryStore::open(&self.path)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let records = store
            .search(query, limit)
            .map_err(|error| ToolError::new(error.to_string()))?;
        serde_json::to_value(records).map_err(|error| ToolError::new(error.to_string()))
    }
}

struct MemorySave {
    path: PathBuf,
}

struct TaskDelegate {
    config: ControlPlaneDelegation,
}

impl Tool for TaskDelegate {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "task.delegate".to_string(),
            description: "Submit an explicitly scoped task to the configured MundusX Control Plane"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string"},
                    "model": {"type": "string"},
                    "max_tokens": {"type": "integer", "minimum": 1, "maximum": 16384},
                    "execution_mode": {"type": "string", "enum": ["single", "decompose"]}
                },
                "required": ["prompt"],
                "additionalProperties": false
            }),
            read_only: false,
        }
    }

    fn execute(&self, arguments: &Value, _: &ToolContext) -> Result<Value, ToolError> {
        let prompt = required_string(arguments, "prompt")?;
        let mut request = ureq::post(&format!(
            "{}/v1/jobs",
            self.config.base_url.trim_end_matches('/')
        ));
        if let Some(token) = &self.config.bearer_token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        request
            .send_json(json!({
                "request_id": format!("agent-{}", uuid::Uuid::new_v4().simple()),
                "prompt": prompt,
                "model": arguments.get("model").and_then(Value::as_str),
                "max_tokens": arguments.get("max_tokens").and_then(Value::as_u64).unwrap_or(1024),
                "max_tokens_source": "agent_explicit",
                "execution_mode": arguments.get("execution_mode").and_then(Value::as_str).unwrap_or("single")
            }))
            .map_err(|error| ToolError::new(format!("control-plane delegation failed: {error}")))?
            .into_json()
            .map_err(|error| ToolError::new(format!("invalid control-plane response: {error}")))
    }
}

impl Tool for MemorySave {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory.save".to_string(),
            description: "Save a durable local memory after user approval".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string"},
                    "tags": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["content"],
                "additionalProperties": false
            }),
            read_only: false,
        }
    }

    fn execute(&self, arguments: &Value, _: &ToolContext) -> Result<Value, ToolError> {
        let content = required_string(arguments, "content")?;
        let tags = arguments
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| ToolError::new("tags must contain strings"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let store = SqliteMemoryStore::open(&self.path)
            .map_err(|error| ToolError::new(error.to_string()))?;
        let record = store
            .save(content, &tags)
            .map_err(|error| ToolError::new(error.to_string()))?;
        serde_json::to_value(record).map_err(|error| ToolError::new(error.to_string()))
    }
}

impl Tool for ValidationRun {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "validation.run".to_string(),
            description: "Run a configured validation profile without invoking a shell".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"profile": {"type": "string"}},
                "required": ["profile"],
                "additionalProperties": false
            }),
            read_only: false,
        }
    }

    fn execute(&self, arguments: &Value, context: &ToolContext) -> Result<Value, ToolError> {
        let name = required_string(arguments, "profile")?;
        let profile = self
            .profiles
            .get(name)
            .ok_or_else(|| ToolError::new(format!("unknown validation profile: {name}")))?;
        let output = Command::new(&profile.program)
            .args(&profile.arguments)
            .current_dir(context.workspace())
            .output()
            .map_err(|error| ToolError::new(format!("could not run profile {name}: {error}")))?;
        Ok(json!({
            "profile": name,
            "success": output.status.success(),
            "exit_code": output.status.code(),
            "stdout": bounded_output(&output.stdout),
            "stderr": bounded_output(&output.stderr)
        }))
    }
}

fn git_definition(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema: json!({"type":"object","additionalProperties":false}),
        read_only: true,
    }
}

fn git_output(context: &ToolContext, args: &[&str]) -> Result<Value, ToolError> {
    git_text(context, args).map(|output| json!({"output": output}))
}

fn git_text(context: &ToolContext, args: &[&str]) -> Result<String, ToolError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(context.workspace())
        .output()
        .map_err(|error| ToolError::new(format!("could not run git: {error}")))?;
    if !output.status.success() {
        return Err(ToolError::new(
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.len() > 256 * 1024 {
        text.truncate(256 * 1024);
        text.push_str("\n[truncated]");
    }
    Ok(text)
}

fn required_string<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, ToolError> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::new(format!("{name} must be a non-empty string")))
}

fn bounded_output(bytes: &[u8]) -> String {
    let mut output = String::from_utf8_lossy(bytes).to_string();
    if output.len() > 256 * 1024 {
        output.truncate(256 * 1024);
        output.push_str("\n[truncated]");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_only_read_only_phase_one_tools() {
        let definitions = read_only_registry().expect("registry").definitions();
        assert_eq!(definitions.len(), 4);
        assert!(definitions.iter().all(|definition| definition.read_only));
    }

    #[test]
    fn rejects_paths_outside_the_workspace() {
        let context = ToolContext::new(std::env::current_dir().expect("cwd")).expect("context");
        let result = FileRead.execute(
            &json!({"path": "../../../../../../Windows/win.ini"}),
            &context,
        );
        assert!(result.is_err());
    }

    #[test]
    fn coding_tools_are_marked_for_approval() {
        let definitions = coding_registry(Default::default())
            .expect("registry")
            .definitions();
        assert_eq!(definitions.len(), 6);
        assert!(
            !definitions
                .iter()
                .find(|definition| definition.name == "patch.apply")
                .expect("patch tool")
                .read_only
        );
    }

    #[test]
    fn memory_save_requires_approval_while_search_is_read_only() {
        let registry = full_registry(Default::default(), std::env::temp_dir().join("memory.db"))
            .expect("registry");
        assert!(
            registry
                .definition("memory.search")
                .expect("search")
                .read_only
        );
        assert!(!registry.definition("memory.save").expect("save").read_only);
    }

    #[test]
    fn control_plane_delegation_is_optional_and_requires_approval() {
        let mut registry = read_only_registry().expect("registry");
        register_control_plane_delegation(
            &mut registry,
            ControlPlaneDelegation {
                base_url: "https://control.example".to_string(),
                bearer_token: None,
            },
        )
        .expect("delegation tool");
        assert!(
            !registry
                .definition("task.delegate")
                .expect("tool")
                .read_only
        );
    }
}
