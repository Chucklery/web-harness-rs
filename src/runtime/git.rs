use super::context::ExecutionContext;
use super::protected;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::git::{self, GitError};
use crate::path_policy;
use crate::permission::{Capability, ExecAuthorization};
use crate::sandbox::NetworkPolicy;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRisk {
    ReadOnly,
    LocalWrite,
    RemoteWrite,
}

impl GitRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::LocalWrite => "local_write",
            Self::RemoteWrite => "remote_write",
        }
    }
}

pub struct GitRuntime;

impl RuntimeTool for GitRuntime {
    fn name(&self) -> &'static str {
        "git"
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
        let risk = risk_for(action).ok_or_else(|| {
            RuntimeToolError::new(RuntimeErrorKind::InvalidArguments, "unknown git action")
        })?;
        let staged = arguments
            .get("staged")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let pathspec = parse_pathspec(arguments)?;
        let message = arguments.get("message").and_then(Value::as_str);
        let branch = arguments.get("branch").and_then(Value::as_str);
        let remote = arguments.get("remote").and_then(Value::as_str);
        let refspec = arguments.get("refspec").and_then(Value::as_str);
        let expected_head = arguments
            .get("expected_head")
            .and_then(Value::as_str)
            .map(validate_expected_head)
            .transpose()?;
        if action == "commit" && expected_head.is_none() {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "commit requires expected_head",
            ));
        }
        if let Some(expected_head) = expected_head.as_deref() {
            ensure_expected_head(context.workspace(), expected_head)?;
        }

        match risk {
            GitRisk::ReadOnly => {
                let protected_paths = match action {
                    "diff" => git::protected_diff_paths(context.workspace(), staged, &pathspec)
                        .map_err(git_error)?,
                    "show_file"
                        if pathspec.first().is_some_and(|path| {
                            path_policy::is_protected_workspace_path(context.workspace(), path)
                        }) =>
                    {
                        pathspec.clone()
                    }
                    _ => Vec::new(),
                };
                if !protected_paths.is_empty() {
                    let argv = if action == "diff" {
                        let mut argv = git::diff_authorization_argv(staged, &pathspec);
                        argv.push("--protected-paths".into());
                        argv.extend(protected_paths.iter().cloned());
                        argv
                    } else {
                        let path = pathspec.first().expect("protected_read has a path");
                        let revision = arguments
                            .get("revision")
                            .and_then(Value::as_str)
                            .unwrap_or("HEAD");
                        vec!["git".into(), "show".into(), format!("{revision}:{path}")]
                    };
                    let authorization = protected::authorization(argv, true);
                    if let Some(pending) = protected::authorize_or_request(
                        context,
                        arguments.get("approval_id").and_then(Value::as_str),
                        &authorization,
                        if action == "diff" {
                            "Read protected files in a Git diff"
                        } else {
                            "Read a protected Git revision file"
                        },
                        &protected_paths,
                    )? {
                        return Ok(pending);
                    }
                } else {
                    context
                        .permissions()
                        .authorize(Capability::GitRead)
                        .map_err(permission_error)?;
                }
            }
            GitRisk::LocalWrite | GitRisk::RemoteWrite => {
                let capability = if risk == GitRisk::RemoteWrite {
                    Capability::GitRemoteWrite
                } else {
                    Capability::GitLocalWrite
                };
                let argv =
                    git::mutation_argv(action, staged, &pathspec, message, branch, remote, refspec)
                        .map_err(git_error)?;
                let authorization = ExecAuthorization {
                    capability,
                    argv,
                    cwd: Some(".".into()),
                    background: false,
                    network: NetworkPolicy::Deny,
                    expected_head: expected_head.clone(),
                    stdin: None,
                    protected_read: false,
                };
                if let Some(approval_id) = arguments.get("approval_id").and_then(Value::as_str) {
                    context
                        .permissions()
                        .consume_exec(approval_id, &authorization)
                        .map_err(permission_error)?;
                } else {
                    let (summary, reason) = if risk == GitRisk::RemoteWrite {
                        (
                            "Push Git commits to a remote".to_string(),
                            "Git push changes a remote repository and requires explicit one-time approval"
                                .to_string(),
                        )
                    } else {
                        (
                            format!("Run structured git {action}"),
                            "Git mutation changes the local repository and requires explicit one-time approval"
                                .to_string(),
                        )
                    };
                    let approval = context
                        .permissions()
                        .request_action(&authorization, summary, reason)
                        .map_err(permission_error)?;
                    return Ok(json!({
                        "status": "approval_required",
                        "approval": approval,
                        "risk": risk.as_str()
                    }));
                }
                if let Some(expected_head) = expected_head.as_deref() {
                    ensure_expected_head(context.workspace(), expected_head)?;
                }
            }
        }

        let workspace = context.workspace();
        if action == "show_file" {
            let path = pathspec.first().ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "show_file requires exactly one pathspec",
                )
            })?;
            if pathspec.len() != 1 {
                return Err(RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "show_file requires exactly one pathspec",
                ));
            }
            let start_line =
                bounded_argument(arguments.get("start_line"), 1, 1_000_000, "start_line")?;
            let max_bytes = bounded_argument(
                arguments.get("max_bytes"),
                256 * 1024,
                256 * 1024,
                "max_bytes",
            )?;
            let page = git::show_file_page(
                workspace,
                arguments
                    .get("revision")
                    .and_then(Value::as_str)
                    .unwrap_or("HEAD"),
                path,
                start_line,
                max_bytes,
            )
            .map_err(git_error)?;
            return serde_json::to_value(page).map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
            });
        }
        let result = match action {
            "head" => {
                let head = git::head(workspace).map_err(git_error)?;
                return Ok(json!({"head": head}));
            }
            "status" => git::status(workspace),
            "diff" => {
                let offset = arguments.get("offset").and_then(Value::as_u64);
                let limit = arguments.get("limit").and_then(Value::as_u64);
                if offset.is_some() || limit.is_some() {
                    let page = git::diff_page(
                        workspace,
                        staged,
                        &pathspec,
                        offset.unwrap_or(0),
                        limit.unwrap_or(16),
                    )
                    .map_err(git_error)?;
                    return serde_json::to_value(page).map_err(|error| {
                        RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
                    });
                } else {
                    git::diff(workspace, staged, &pathspec)
                }
            }
            "log" => git::log(
                workspace,
                arguments.get("limit").and_then(Value::as_u64).unwrap_or(20),
            ),
            "show" => git::show(
                workspace,
                arguments
                    .get("revision")
                    .and_then(Value::as_str)
                    .unwrap_or("HEAD"),
            ),
            "add" => git::add(workspace, &pathspec),
            "commit" => git::commit(workspace, message.unwrap_or_default(), &pathspec),
            "switch" => git::switch(workspace, branch.unwrap_or_default()),
            "create_branch" => git::create_branch(workspace, branch.unwrap_or_default()),
            "restore" => git::restore(workspace, staged, &pathspec),
            "push" => git::push(workspace, remote, refspec),
            _ => unreachable!("risk_for validates the action"),
        }
        .map_err(git_error)?;

        serde_json::to_value(result)
            .map_err(|error| RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string()))
    }
}

fn ensure_expected_head(
    workspace: &crate::workspace::Workspace,
    expected_head: &str,
) -> Result<(), RuntimeToolError> {
    let actual_head = git::head(workspace).map_err(git_error)?;
    verify_expected_head(expected_head, &actual_head)
}

fn verify_expected_head(expected_head: &str, actual_head: &str) -> Result<(), RuntimeToolError> {
    if actual_head != expected_head {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::Conflict,
            "repository HEAD changed since expected_head was supplied",
        ));
    }
    Ok(())
}

fn risk_for(action: &str) -> Option<GitRisk> {
    match action {
        "head" | "status" | "diff" | "log" | "show" | "show_file" => Some(GitRisk::ReadOnly),
        "add" | "commit" | "switch" | "create_branch" | "restore" => Some(GitRisk::LocalWrite),
        "push" => Some(GitRisk::RemoteWrite),
        _ => None,
    }
}

fn parse_pathspec(arguments: &Value) -> Result<Vec<String>, RuntimeToolError> {
    let Some(values) = arguments.get("pathspec") else {
        return Ok(Vec::new());
    };
    let values = values.as_array().ok_or_else(|| {
        RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "pathspec must be an array",
        )
    })?;
    if values.len() > 32 {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "pathspec may contain at most 32 paths",
        ));
    }
    values
        .iter()
        .map(|value| {
            value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "pathspec items must be strings",
                )
            })
        })
        .collect()
}

fn validate_expected_head(value: &str) -> Result<String, RuntimeToolError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_whitespace) {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "expected_head must be a non-empty revision without whitespace",
        ));
    }
    Ok(value.to_string())
}

fn bounded_argument(
    value: Option<&Value>,
    default: usize,
    maximum: usize,
    name: &str,
) -> Result<usize, RuntimeToolError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value.as_u64().ok_or_else(|| {
        RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            format!("{name} must be an unsigned integer"),
        )
    })? as usize;
    if parsed == 0 || parsed > maximum {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            format!("{name} must be 1..={maximum}"),
        ));
    }
    Ok(parsed)
}

fn git_error(error: GitError) -> RuntimeToolError {
    let kind = match &error {
        GitError::Invalid(_) => RuntimeErrorKind::InvalidArguments,
        GitError::Limit(_) => RuntimeErrorKind::LimitExceeded,
        GitError::Workspace(crate::workspace::WorkspaceError::Denied(_)) => {
            RuntimeErrorKind::Denied
        }
        GitError::Workspace(crate::workspace::WorkspaceError::Io(io_error))
            if io_error.kind() == std::io::ErrorKind::NotFound =>
        {
            RuntimeErrorKind::NotFound
        }
        GitError::Workspace(_) => RuntimeErrorKind::Workspace,
        GitError::Unavailable => RuntimeErrorKind::Dependency,
        GitError::Failed(_) => RuntimeErrorKind::Execution,
    };
    let message = match error {
        GitError::Invalid(message) => message,
        other => other.to_string(),
    };
    RuntimeToolError::new(kind, message)
}

fn permission_error(error: crate::permission::PermissionError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use std::fs;
    use std::process::Command;

    #[test]
    fn git_actions_have_explicit_risk_classes() {
        assert_eq!(risk_for("head"), Some(GitRisk::ReadOnly));
        assert_eq!(risk_for("status"), Some(GitRisk::ReadOnly));
        assert_eq!(risk_for("show_file"), Some(GitRisk::ReadOnly));
        assert_eq!(risk_for("commit"), Some(GitRisk::LocalWrite));
        assert_eq!(risk_for("create_branch"), Some(GitRisk::LocalWrite));
        assert_eq!(risk_for("push"), Some(GitRisk::RemoteWrite));
        assert_eq!(risk_for("unknown"), None);
    }

    #[test]
    fn head_action_returns_full_commit_id() {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        }
        fs::write(dir.path().join("file.txt"), "one\n").unwrap();
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        crate::git::add(&workspace, &["file.txt".into()]).unwrap();
        crate::git::commit(&workspace, "initial", &[]).unwrap();
        let expected = crate::git::head(&workspace).unwrap();

        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let result = GitRuntime
            .call(
                &mut ExecutionContext::new(&workspace, &mut jobs, &mut permissions),
                &json!({"action":"head"}),
            )
            .unwrap();
        assert_eq!(result["head"], expected);
    }

    #[test]
    fn malformed_pathspec_is_rejected() {
        let error = parse_pathspec(&json!({"pathspec": ["ok", 3]})).unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn commit_requires_expected_head_before_requesting_approval() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success());
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();

        let error = GitRuntime
            .call(
                &mut ExecutionContext::new(&workspace, &mut jobs, &mut permissions),
                &json!({"action":"commit","message":"test"}),
            )
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
        assert!(error.message().contains("expected_head"));
    }

    #[test]
    fn diff_requires_host_approval_before_returning_protected_content() {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        }
        fs::write(dir.path().join(".env"), "TOKEN=old-secret\n").unwrap();
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        crate::git::add(&workspace, &[".env".into()]).unwrap();
        crate::git::commit(&workspace, "add protected file", &[]).unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=new-secret\n").unwrap();

        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let request = json!({"action":"diff"});
        let pending = GitRuntime
            .call(
                &mut ExecutionContext::new(&workspace, &mut jobs, &mut permissions),
                &request,
            )
            .unwrap();
        assert_eq!(pending["status"], "approval_required");
        assert_eq!(pending["protected_paths"], json!([".env"]));
        assert!(!pending.to_string().contains("new-secret"));

        let approval_id = pending["approval"]["id"].as_str().unwrap().to_string();
        permissions.approve(&approval_id).unwrap();
        let mut approved_request = request.as_object().unwrap().clone();
        approved_request.insert("approval_id".into(), Value::String(approval_id));
        let result = GitRuntime
            .call(
                &mut ExecutionContext::new(&workspace, &mut jobs, &mut permissions),
                &Value::Object(approved_request),
            )
            .unwrap();
        assert!(result["stdout"]
            .as_str()
            .unwrap()
            .contains("+TOKEN=[REDACTED]"));
    }

    #[test]
    fn missing_git_is_reported_as_a_dependency_problem() {
        let error = git_error(GitError::Unavailable);
        assert_eq!(error.kind(), RuntimeErrorKind::Dependency);
    }

    #[test]
    fn maps_git_file_capture_limit_to_limit_error() {
        let error = git_error(GitError::Limit("blob too large".into()));
        assert_eq!(error.kind(), RuntimeErrorKind::LimitExceeded);
    }

    #[test]
    fn show_file_paging_arguments_are_bounded() {
        assert_eq!(bounded_argument(None, 1, 100, "start_line").unwrap(), 1);
        assert_eq!(
            bounded_argument(Some(&json!(100)), 1, 100, "start_line").unwrap(),
            100
        );
        assert_eq!(
            bounded_argument(Some(&json!(0)), 1, 100, "start_line")
                .unwrap_err()
                .kind(),
            RuntimeErrorKind::InvalidArguments
        );
        assert_eq!(
            bounded_argument(Some(&json!(101)), 1, 100, "start_line")
                .unwrap_err()
                .kind(),
            RuntimeErrorKind::InvalidArguments
        );
    }

    #[test]
    fn failed_git_commands_stay_execution_errors() {
        let error = git_error(GitError::Failed("boom".to_string()));
        assert_eq!(error.kind(), RuntimeErrorKind::Execution);
    }

    #[test]
    fn workspace_git_errors_keep_boundary_classification() {
        let denied = git_error(GitError::Workspace(
            crate::workspace::WorkspaceError::Denied("/workspace/.git".into()),
        ));
        assert_eq!(denied.kind(), RuntimeErrorKind::Denied);

        let missing = git_error(GitError::Workspace(crate::workspace::WorkspaceError::Io(
            std::io::Error::from(std::io::ErrorKind::NotFound),
        )));
        assert_eq!(missing.kind(), RuntimeErrorKind::NotFound);
    }

    #[test]
    fn expected_head_rejects_whitespace() {
        let error = validate_expected_head("abc 123").unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn expected_head_accepts_revision_tokens() {
        assert_eq!(validate_expected_head("abc123").unwrap(), "abc123");
    }

    #[test]
    fn expected_head_guard_rejects_changed_head() {
        assert_eq!(
            verify_expected_head("abc123", "def456").unwrap_err().kind(),
            RuntimeErrorKind::Conflict
        );
    }
}
