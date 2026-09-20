use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::patch;
use crate::permission::Capability;
use serde_json::{json, Value};

const MAX_PATCH_BYTES: usize = 512 * 1024;

pub struct PatchRuntime;

impl RuntimeTool for PatchRuntime {
    fn name(&self) -> &'static str {
        "patch"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        context
            .permissions()
            .authorize(Capability::WorkspaceWrite)
            .map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
            })?;
        let patch_text = arguments
            .get("patch")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "arguments.patch is required",
                )
            })?;
        if patch_text.len() > MAX_PATCH_BYTES {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::LimitExceeded,
                "patch exceeds 512 KiB",
            ));
        }
        let changed_paths = patch::apply(context.workspace(), patch_text).map_err(|error| {
            RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
        })?;
        Ok(json!({"changed_paths": changed_paths}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;

    #[test]
    fn patch_requires_payload() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let error = PatchRuntime.call(&mut context, &json!({})).unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}
