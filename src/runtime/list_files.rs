use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::env;
use crate::path_policy;
use crate::permission::Capability;
use crate::process;
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::process::{Command, Stdio};

const MAX_INCLUDE_BYTES: usize = 256;
const MAX_TRACKED_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCANNED_ENTRIES: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    File,
    Directory,
    Symlink,
}

impl FileKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
        }
    }
}

#[derive(Debug)]
struct Entry {
    path: String,
    kind: FileKind,
}

pub struct ListFilesRuntime;

impl RuntimeTool for ListFilesRuntime {
    fn name(&self) -> &'static str {
        "list_files"
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
        let source = arguments
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("tracked");
        if !matches!(source, "tracked" | "all") {
            return Err(invalid("source must be tracked or all"));
        }
        let offset = parse_offset(arguments.get("offset"))?;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100);
        let limits = context.limits();
        if limit == 0 || limit as usize > limits.max_list_files {
            return Err(invalid("limit must be 1..=200"));
        }
        let include = arguments.get("include").and_then(Value::as_str);
        if include.is_some_and(|value| value.len() > MAX_INCLUDE_BYTES) {
            return Err(invalid("include exceeds 256 bytes"));
        }

        let entries = match source {
            "tracked" => tracked_entries(context, path, include)?,
            "all" => all_entries(context, path, include)?,
            _ => unreachable!(),
        };
        let source_complete = entries.1;
        let entries = entries.0;
        let start = offset.min(entries.len());
        let end = (start + limit as usize).min(entries.len());
        let items = entries[start..end]
            .iter()
            .map(|entry| json!({"path": entry.path, "type": entry.kind.as_str()}))
            .collect::<Vec<_>>();
        let page_complete = end == entries.len();
        let truncated = !source_complete || !page_complete;
        let next_offset = if source_complete && !page_complete {
            Some(end)
        } else {
            None
        };

        Ok(json!({
            "source": source,
            "items": items,
            "next_offset": next_offset,
            "truncated": truncated
        }))
    }
}

fn tracked_entries(
    context: &ExecutionContext<'_>,
    path: &str,
    include: Option<&str>,
) -> Result<(Vec<Entry>, bool), RuntimeToolError> {
    let relative = validate_directory(context, path)?;
    let root = context.workspace().root().display().to_string();
    let mut command = Command::new("git");
    command.args(["-C", &root, "ls-files", "-z"]);
    if relative != "." {
        command.args(["--", &relative]);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    env::apply(&mut command);
    let mut child = process::spawn_in_own_group(&mut command).map_err(|error| {
        RuntimeToolError::new(
            RuntimeErrorKind::Dependency,
            format!("git is unavailable: {error}"),
        )
    })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| execution("git stdout unavailable"))?;
    let mut bytes = Vec::with_capacity(MAX_TRACKED_BYTES.min(64 * 1024));
    let mut buffer = [0u8; 8192];
    let mut complete = true;
    loop {
        let read = stdout
            .read(&mut buffer)
            .map_err(|error| execution(error.to_string()))?;
        if read == 0 {
            break;
        }
        let remaining = MAX_TRACKED_BYTES.saturating_sub(bytes.len());
        if read > remaining {
            bytes.extend_from_slice(&buffer[..remaining]);
            complete = false;
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    if !complete {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|error| execution(error.to_string()))?;
    if !status.success() && complete {
        return Err(execution("git ls-files failed"));
    }
    let mut entries = bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .filter_map(|path| std::str::from_utf8(path).ok())
        .filter(|path| {
            !path_policy::is_protected_workspace_path(context.workspace(), path)
                && include
                    .map(|pattern| glob_matches(pattern, path))
                    .unwrap_or(true)
        })
        .filter_map(|path| entry_for_relative(context, path).ok())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((entries, complete))
}

fn all_entries(
    context: &ExecutionContext<'_>,
    path: &str,
    include: Option<&str>,
) -> Result<(Vec<Entry>, bool), RuntimeToolError> {
    let root = validate_directory(context, path)?;
    let absolute = context
        .workspace()
        .resolve(&root)
        .map_err(workspace_error)?;
    let mut pending = vec![absolute];
    let mut entries = Vec::new();
    let mut scanned = 0usize;
    let mut complete = true;
    while let Some(directory) = pending.pop() {
        let read_dir = fs::read_dir(&directory).map_err(|error| execution(error.to_string()))?;
        for item in read_dir {
            scanned += 1;
            if scanned > MAX_SCANNED_ENTRIES {
                complete = false;
                break;
            }
            let item = item.map_err(|error| execution(error.to_string()))?;
            let item_path = item.path();
            let relative = item_path
                .strip_prefix(context.workspace().root())
                .map_err(|_| execution("enumerated path escaped workspace"))?;
            let relative_text = relative.display().to_string();
            let metadata =
                fs::symlink_metadata(&item_path).map_err(|error| execution(error.to_string()))?;
            let kind = if metadata.file_type().is_symlink() {
                FileKind::Symlink
            } else if metadata.is_dir() {
                FileKind::Directory
            } else {
                FileKind::File
            };
            if !path_policy::is_protected_workspace_path(context.workspace(), &relative_text)
                && include
                    .map(|pattern| glob_matches(pattern, &relative_text))
                    .unwrap_or(true)
                && context.workspace().resolve(&relative_text).is_ok()
            {
                entries.push(Entry {
                    path: relative_text.clone(),
                    kind,
                });
            }
            if kind == FileKind::Directory
                && !path_policy::is_protected_workspace_path(context.workspace(), &relative_text)
                && context.workspace().resolve(&relative_text).is_ok()
            {
                pending.push(item_path);
            }
        }
        if !complete {
            break;
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((entries, complete))
}

fn validate_directory(
    context: &ExecutionContext<'_>,
    path: &str,
) -> Result<String, RuntimeToolError> {
    let resolved = context.workspace().resolve(path).map_err(workspace_error)?;
    if !resolved.is_dir() {
        return Err(invalid("path must be a directory"));
    }
    Ok(path.trim_matches('/').to_string().replace('\\', "/"))
}

fn entry_for_relative(
    context: &ExecutionContext<'_>,
    path: &str,
) -> Result<Entry, RuntimeToolError> {
    let resolved = context.workspace().resolve(path).map_err(workspace_error)?;
    let raw_path = context.workspace().root().join(path);
    let metadata = fs::symlink_metadata(&raw_path)
        .or_else(|_| fs::symlink_metadata(&resolved))
        .map_err(|error| execution(error.to_string()))?;
    let kind = if metadata.file_type().is_symlink() {
        FileKind::Symlink
    } else if metadata.is_dir() {
        FileKind::Directory
    } else {
        FileKind::File
    };
    Ok(Entry {
        path: path.to_string(),
        kind,
    })
}

fn parse_offset(value: Option<&Value>) -> Result<usize, RuntimeToolError> {
    let value = value.and_then(Value::as_u64).unwrap_or(0);
    usize::try_from(value).map_err(|_| invalid("offset is too large"))
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let (mut pattern_index, mut value_index) = (0, 0);
    let mut star = None;
    let mut after_star = 0;
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            after_star = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            after_star += 1;
            value_index = after_star;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
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

fn execution(message: impl Into<String>) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Execution, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;

    fn context<'a>(
        workspace: &'a Workspace,
        jobs: &'a mut JobManager,
        permissions: &'a mut PermissionEngine,
    ) -> ExecutionContext<'a> {
        ExecutionContext::new(workspace, jobs, permissions)
    }

    #[test]
    fn lists_sorted_all_entries_with_continuation() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        fs::write(dir.path().join("src/a.rs"), "a").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let value = ListFilesRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &json!({"source":"all", "limit": 1}),
            )
            .unwrap();
        assert_eq!(value["items"][0]["path"], "b.txt");
        assert_eq!(value["next_offset"], 1);
        assert_eq!(value["truncated"], true);
    }

    #[test]
    fn rejects_file_as_listing_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let error = ListFilesRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &json!({"path":"file.txt"}),
            )
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn filters_protected_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".ssh")).unwrap();
        fs::create_dir(dir.path().join("secrets")).unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=secret").unwrap();
        fs::write(dir.path().join(".ssh/id_ed25519"), "private").unwrap();
        fs::write(dir.path().join("secrets/config.json"), "private").unwrap();
        fs::write(dir.path().join("visible.txt"), "visible").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(".env", dir.path().join("config.txt")).unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let value = ListFilesRuntime
            .call(
                &mut context(&workspace, &mut jobs, &mut permissions),
                &json!({"source":"all"}),
            )
            .unwrap();
        let paths = value["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["path"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["visible.txt"]);
    }
}
