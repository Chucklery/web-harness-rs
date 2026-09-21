use super::context::ExecutionContext;
use super::protected;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::path_policy;
use crate::permission::Capability;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, BufReader};

const MAX_LINE_BYTES: usize = 1024 * 1024;

pub struct FileRuntime;

#[derive(Debug)]
struct ReadRequest {
    path: String,
    start_line: usize,
    end_line: Option<usize>,
    expected_revision: Option<String>,
}

struct ReadResult {
    text: String,
    end_line: usize,
    next_start_line: usize,
    truncated: bool,
    revision: String,
}

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
            .map_err(permission_error)?;
        let values = arguments
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("arguments.paths is required"))?;
        let limits = context.limits();
        if values.is_empty() || values.len() > limits.max_read_paths {
            return Err(invalid("paths must contain 1..=16 items"));
        }

        let requests = values
            .iter()
            .map(parse_request)
            .collect::<Result<Vec<_>, _>>()?;
        if requests
            .iter()
            .any(|request| path_policy::is_protected(&request.path))
        {
            let authorization = sensitive_read_authorization(&requests);
            let protected_paths = requests
                .iter()
                .filter(|request| path_policy::is_protected(&request.path))
                .map(|request| request.path.clone())
                .collect::<Vec<_>>();
            if let Some(pending) = protected::authorize_or_request(
                context,
                arguments.get("approval_id").and_then(Value::as_str),
                &authorization,
                "Read protected workspace files",
                &protected_paths,
            )? {
                return Ok(pending);
            }
        }

        let mut total = 0usize;
        let mut files = Vec::with_capacity(values.len());
        for request in &requests {
            let read = read_range(context, request, limits.max_read_file_bytes)?;
            total = total.checked_add(read.text.len()).ok_or_else(|| {
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
            let next = if read.truncated {
                Some(json!({
                    "path": request.path,
                    "start_line": read.next_start_line,
                    "end_line": request.end_line,
                    "expected_read_revision": read.revision
                }))
            } else {
                None
            };
            files.push(json!({
                "path": request.path,
                "text": read.text,
                "start_line": request.start_line,
                "end_line": read.end_line,
                "truncated": read.truncated,
                "next": next,
                "read_revision": read.revision
            }));
        }
        Ok(json!({"files": files}))
    }
}

fn sensitive_read_authorization(requests: &[ReadRequest]) -> crate::permission::ExecAuthorization {
    let mut argv = vec!["read_files".to_string()];
    for request in requests {
        argv.push(request.path.clone());
        argv.push(request.start_line.to_string());
        argv.push(
            request
                .end_line
                .map_or_else(|| "-".into(), |line| line.to_string()),
        );
        argv.push(request.expected_revision.clone().unwrap_or_default());
    }
    protected::authorization(argv, false)
}

fn parse_request(value: &Value) -> Result<ReadRequest, RuntimeToolError> {
    if let Some(path) = value.as_str() {
        return Ok(ReadRequest {
            path: path.to_string(),
            start_line: 1,
            end_line: None,
            expected_revision: None,
        });
    }
    let object = value
        .as_object()
        .ok_or_else(|| invalid("path items must be strings or objects"))?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("path is required"))?;
    let start = object
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if start == 0 {
        return Err(invalid("start_line must be at least 1"));
    }
    let end = object.get("end_line").and_then(Value::as_u64);
    if end.is_some_and(|value| value < start) {
        return Err(invalid(
            "end_line must be greater than or equal to start_line",
        ));
    }
    Ok(ReadRequest {
        path: path.to_string(),
        start_line: usize::try_from(start).map_err(|_| invalid("start_line is too large"))?,
        end_line: end
            .map(|value| usize::try_from(value).map_err(|_| invalid("end_line is too large")))
            .transpose()?,
        expected_revision: object
            .get("expected_read_revision")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

fn read_range(
    context: &ExecutionContext<'_>,
    request: &ReadRequest,
    max_bytes: usize,
) -> Result<ReadResult, RuntimeToolError> {
    let workspace = context.workspace();
    let revision = workspace
        .read_revision(&request.path)
        .map_err(workspace_error)?;
    if request
        .expected_revision
        .as_deref()
        .is_some_and(|expected| expected != revision)
    {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::Conflict,
            "file changed since expected_read_revision was created",
        ));
    }

    let path = workspace.resolve(&request.path).map_err(workspace_error)?;
    if !path.is_file() {
        return Err(invalid("path is not a regular file"));
    }
    let file = File::open(path).map_err(|error| workspace_error(error.into()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_number = 0usize;
    let mut end_line = request.start_line.saturating_sub(1);
    let mut text = String::new();
    let mut truncated = false;
    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| workspace_error(error.into()))?;
        if bytes == 0 {
            break;
        }
        line_number += 1;
        if bytes > MAX_LINE_BYTES {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::LimitExceeded,
                "line exceeds 1 MiB read limit",
            ));
        }
        if line_number < request.start_line {
            continue;
        }
        if request.end_line.is_some_and(|end| line_number > end) {
            break;
        }
        if text.len() + line.len() > max_bytes {
            truncated = true;
            break;
        }
        text.push_str(&line);
        end_line = line_number;
    }
    Ok(ReadResult {
        text,
        end_line,
        next_start_line: end_line.saturating_add(1),
        truncated,
        revision,
    })
}

fn permission_error(error: crate::permission::PermissionError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
}

fn workspace_error(error: crate::workspace::WorkspaceError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Workspace, error.to_string())
}

fn invalid(message: impl Into<String>) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::InvalidArguments, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;
    use std::fs;

    fn context<'a>(
        workspace: &'a Workspace,
        jobs: &'a mut JobManager,
        permissions: &'a mut PermissionEngine,
    ) -> ExecutionContext<'a> {
        ExecutionContext::new(workspace, jobs, permissions)
    }

    #[test]
    fn reads_ranges_and_returns_revision() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let value = FileRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &json!({"paths": [{"path":"a.txt", "start_line":2, "end_line":2}]}),
            )
            .unwrap();
        assert_eq!(value["files"][0]["text"], "two\n");
        assert!(value["files"][0]["read_revision"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }

    #[test]
    fn rejects_stale_continuation_revision() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let revision = workspace.read_revision("a.txt").unwrap();
        fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let error = FileRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &json!({"paths": [{"path":"a.txt", "expected_read_revision":revision}]}),
            )
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::Conflict);
    }

    #[test]
    fn keeps_legacy_string_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "alpha").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let value = FileRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &json!({"paths": ["a.txt"]}),
            )
            .unwrap();
        assert_eq!(value["files"][0]["text"], "alpha");
    }

    #[test]
    fn protected_reads_require_and_consume_approval() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=secret\n").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let request = json!({"paths": [".env"]});
        let pending = FileRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &request,
            )
            .unwrap();
        assert_eq!(pending["status"], "approval_required");
        let approval_id = pending["approval"]["id"].as_str().unwrap().to_string();
        permissions.approve(&approval_id).unwrap();

        let mut approved = request.as_object().unwrap().clone();
        approved.insert("approval_id".into(), Value::String(approval_id));
        let value = FileRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &Value::Object(approved),
            )
            .unwrap();
        assert_eq!(value["files"][0]["text"], "TOKEN=secret\n");
    }
}
