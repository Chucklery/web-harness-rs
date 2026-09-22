use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::patch;
use crate::permission::Capability;
use serde_json::{json, Value};
use std::collections::HashMap;

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
        let expected_revisions =
            parse_expected_revisions(arguments.get("expected_read_revisions"))?;
        let changed_paths =
            patch::apply_with_revisions(context.workspace(), patch_text, &expected_revisions)
                .map_err(|error| {
                    let kind = patch_error_kind(&error);
                    RuntimeToolError::new(kind, error.to_string())
                })?;
        Ok(json!({"changed_paths": changed_paths}))
    }
}

fn patch_error_kind(error: &patch::PatchError) -> RuntimeErrorKind {
    match error {
        patch::PatchError::Invalid(_) => RuntimeErrorKind::InvalidArguments,
        patch::PatchError::Conflict(_) => RuntimeErrorKind::Conflict,
        patch::PatchError::Workspace(crate::workspace::WorkspaceError::Denied(_)) => {
            RuntimeErrorKind::Denied
        }
        patch::PatchError::Workspace(crate::workspace::WorkspaceError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            RuntimeErrorKind::NotFound
        }
        patch::PatchError::Workspace(_) => RuntimeErrorKind::Workspace,
        patch::PatchError::Io(_) => RuntimeErrorKind::Execution,
    }
}

fn parse_expected_revisions(
    value: Option<&Value>,
) -> Result<HashMap<String, String>, RuntimeToolError> {
    let Some(object) = value else {
        return Ok(HashMap::new());
    };
    let object = object.as_object().ok_or_else(|| {
        RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "expected_read_revisions must be an object",
        )
    })?;
    if object.len() > 32 {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "expected_read_revisions may contain at most 32 paths",
        ));
    }
    object
        .iter()
        .map(|(path, revision)| {
            let revision = revision.as_str().ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "expected_read_revisions values must be strings",
                )
            })?;
            Ok((path.clone(), revision.to_string()))
        })
        .collect()
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

    #[test]
    fn maps_invalid_patch_syntax_to_invalid_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let error = PatchRuntime
            .call(
                &mut context,
                &json!({"patch": "*** Begin Patch\nnot an operation\n*** End Patch"}),
            )
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn maps_denied_patch_paths_to_denied() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("blocked")).unwrap();
        let workspace = Workspace::with_denied(dir.path(), &["blocked".to_string()]).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let error = PatchRuntime
            .call(
                &mut context,
                &json!({"patch": "*** Begin Patch\n*** Add File: blocked/new.txt\n+data\n*** End Patch"}),
            )
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::Denied);
    }
}
