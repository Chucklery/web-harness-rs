use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::permission::Capability;
use serde_json::{json, Value};

const MAX_INSTRUCTION_FILE_BYTES: usize = 128 * 1024;

pub struct WorkspaceInfoRuntime;

impl RuntimeTool for WorkspaceInfoRuntime {
    fn name(&self) -> &'static str {
        "workspace_info"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        _arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        context
            .permissions()
            .authorize(Capability::WorkspaceRead)
            .map_err(permission_error)?;
        serde_json::to_value(context.workspace().info())
            .map_err(|error| RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string()))
    }
}

pub struct WorkspaceInstructionsRuntime;

impl RuntimeTool for WorkspaceInstructionsRuntime {
    fn name(&self) -> &'static str {
        "workspace_instructions"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        context
            .permissions()
            .authorize(Capability::WorkspaceRead)
            .map_err(permission_error)?;
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let workspace = context.workspace();
        let files = workspace.discover_agents(path).map_err(|error| {
            RuntimeToolError::new(RuntimeErrorKind::Workspace, error.to_string())
        })?;
        let mut instructions = Vec::with_capacity(files.len());
        for file in files {
            let relative = file
                .strip_prefix(workspace.root())
                .unwrap_or(&file)
                .display()
                .to_string();
            let text = workspace
                .read_text_bounded(&relative, MAX_INSTRUCTION_FILE_BYTES)
                .map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Workspace, error.to_string())
                })?;
            instructions.push(json!({"path": relative, "text": text}));
        }
        Ok(json!({"instructions": instructions}))
    }
}

fn permission_error(error: crate::permission::PermissionError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;
    use std::fs;

    #[test]
    fn workspace_info_uses_context_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let value = WorkspaceInfoRuntime.call(&mut context, &json!({})).unwrap();
        assert_eq!(value["root"], workspace.root().display().to_string());
    }

    #[test]
    fn instructions_are_scoped_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "rules").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let value = WorkspaceInstructionsRuntime
            .call(&mut context, &json!({"path":"."}))
            .unwrap();
        assert_eq!(value["instructions"][0]["text"], "rules");
    }
}
