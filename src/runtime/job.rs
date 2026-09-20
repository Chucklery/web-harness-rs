use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::permission::Capability;
use serde_json::{json, Value};

pub struct JobRuntime;

impl RuntimeTool for JobRuntime {
    fn name(&self) -> &'static str {
        "job"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        context
            .permissions()
            .authorize(Capability::JobControl)
            .map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
            })?;

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
            "poll" => serde_json::to_value(context.jobs().poll(id).map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
            })?)
            .map_err(|error| RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())),
            "cancel" => serde_json::to_value(context.jobs().cancel(id).map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
            })?)
            .map_err(|error| RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())),
            "output" => {
                let stream = arguments
                    .get("stream")
                    .and_then(Value::as_str)
                    .unwrap_or("stdout");
                let (text, truncated) = context.jobs().output(id, stream).map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
                })?;
                Ok(json!({"stream": stream, "text": text, "truncated": truncated}))
            }
            _ => Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "unknown job action",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;

    #[test]
    fn rejects_unknown_job_action() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let error = JobRuntime
            .call(&mut context, &json!({"action": "unknown", "id": "job_x"}))
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}
