use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use serde_json::{json, Value};

pub struct PermissionRuntime;

impl RuntimeTool for PermissionRuntime {
    fn name(&self) -> &'static str {
        "permission"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "arguments.action is required",
                )
            })?;
        let id = arguments.get("id").and_then(Value::as_str).ok_or_else(|| {
            RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "arguments.id is required",
            )
        })?;
        match action {
            "approve" => context
                .permissions()
                .approve(id)
                .map_err(permission_error)?,
            "deny" => context.permissions().deny(id).map_err(permission_error)?,
            _ => {
                return Err(RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "unknown permission action",
                ))
            }
        }
        Ok(json!({"status":"ok","id":id}))
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

    #[test]
    fn rejects_unknown_action() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let error = PermissionRuntime
            .call(&mut context, &json!({"action":"unknown","id":"apr_x"}))
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}
