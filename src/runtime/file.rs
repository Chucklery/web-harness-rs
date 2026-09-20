use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::permission::Capability;
use serde_json::{json, Value};

pub struct FileRuntime;

impl RuntimeTool for FileRuntime {
    fn name(&self) -> &'static str {
        "read_files"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        context
            .permissions()
            .authorize(Capability::WorkspaceRead)
            .map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
            })?;
        let limits = context.limits();
        let paths = arguments
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "arguments.paths is required",
                )
            })?;

        if paths.is_empty() || paths.len() > limits.max_read_paths {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "paths must contain 1..=16 items",
            ));
        }

        let mut total = 0usize;
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let path = path.as_str().ok_or_else(|| {
                RuntimeToolError::new(RuntimeErrorKind::InvalidArguments, "path must be a string")
            })?;
            let text = context
                .workspace()
                .read_text_bounded(path, limits.max_read_file_bytes)
                .map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Workspace, error.to_string())
                })?;
            total = total.checked_add(text.len()).ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::LimitExceeded,
                    "batch read exceeds 512 KiB",
                )
            })?;
            if total > limits.max_read_batch_bytes {
                return Err(RuntimeToolError::new(
                    RuntimeErrorKind::LimitExceeded,
                    "batch read exceeds 512 KiB",
                ));
            }
            files.push(json!({"path": path, "text": text}));
        }

        Ok(json!({"files": files}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_files_through_runtime_boundary() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "alpha").unwrap();
        fs::write(dir.path().join("b.txt"), "beta").unwrap();
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        let mut jobs = crate::jobs::JobManager::new();
        let mut permissions = crate::permission::PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let value = FileRuntime
            .call(&mut context, &json!({"paths": ["a.txt", "b.txt"]}))
            .unwrap();

        assert_eq!(value["files"][0]["text"], "alpha");
        assert_eq!(value["files"][1]["text"], "beta");
    }

    #[test]
    fn rejects_invalid_path_batches() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        let mut jobs = crate::jobs::JobManager::new();
        let mut permissions = crate::permission::PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let error = FileRuntime
            .call(&mut context, &json!({"paths": []}))
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}
