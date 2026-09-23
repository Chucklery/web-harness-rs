use super::context::ExecutionContext;
use super::protected;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::path_policy;
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
        let files = context.workspace().discover_agents(path).map_err(|error| {
            RuntimeToolError::new(RuntimeErrorKind::Workspace, error.to_string())
        })?;
        let protected_paths = files
            .iter()
            .filter_map(|file| {
                let workspace = context.workspace();
                let relative = file.strip_prefix(workspace.root()).ok()?;
                path_policy::is_protected_workspace_path(workspace, relative)
                    .then(|| relative.display().to_string())
            })
            .collect::<Vec<_>>();
        if !protected_paths.is_empty() {
            let mut argv = vec!["workspace_instructions".to_string(), path.to_string()];
            argv.extend(protected_paths.iter().cloned());
            let authorization = protected::authorization(argv, true);
            if let Some(pending) = protected::authorize_or_request(
                context,
                arguments.get("approval_id").and_then(Value::as_str),
                &authorization,
                "Read protected workspace instructions",
                &protected_paths,
            )? {
                return Ok(pending);
            }
        }
        let workspace = context.workspace();
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

    #[cfg(unix)]
    #[test]
    fn protected_symlink_instruction_requires_approval() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=secret\n").unwrap();
        std::os::unix::fs::symlink(".env", dir.path().join("AGENTS.md")).unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let result = WorkspaceInstructionsRuntime
            .call(&mut context, &json!({"path":"."}))
            .unwrap();

        assert_eq!(result["status"], "approval_required");
        assert_eq!(result["protected_paths"], json!(["AGENTS.md"]));
        assert!(!result.to_string().contains("TOKEN=secret"));
    }
}
