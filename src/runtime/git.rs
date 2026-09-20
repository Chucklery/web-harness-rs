use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::git::{self, GitError};
use crate::permission::{Capability, ExecAuthorization};
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

        match risk {
            GitRisk::ReadOnly => context
                .permissions()
                .authorize(Capability::GitRead)
                .map_err(permission_error)?,
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
                    let approval =
                        context
                            .permissions()
                            .request_action(&authorization, summary, reason);
                    return Ok(json!({
                        "status": "approval_required",
                        "approval": approval,
                        "risk": risk.as_str()
                    }));
                }
            }
        }

        let workspace = context.workspace();
        let result = match action {
            "status" => git::status(workspace),
            "diff" => git::diff(workspace, staged, &pathspec),
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
            "commit" => git::commit(workspace, message.unwrap_or_default()),
            "switch" => git::switch(workspace, branch.unwrap_or_default()),
            "restore" => git::restore(workspace, staged, &pathspec),
            "push" => git::push(workspace, remote, refspec),
            _ => unreachable!("risk_for validates the action"),
        }
        .map_err(git_error)?;

        serde_json::to_value(result)
            .map_err(|error| RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string()))
    }
}

fn risk_for(action: &str) -> Option<GitRisk> {
    match action {
        "status" | "diff" | "log" | "show" => Some(GitRisk::ReadOnly),
        "add" | "commit" | "switch" | "restore" => Some(GitRisk::LocalWrite),
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

fn git_error(error: GitError) -> RuntimeToolError {
    match error {
        GitError::Invalid(message) => {
            RuntimeToolError::new(RuntimeErrorKind::InvalidArguments, message)
        }
        other => RuntimeToolError::new(RuntimeErrorKind::Execution, other.to_string()),
    }
}

fn permission_error(error: crate::permission::PermissionError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_actions_have_explicit_risk_classes() {
        assert_eq!(risk_for("status"), Some(GitRisk::ReadOnly));
        assert_eq!(risk_for("commit"), Some(GitRisk::LocalWrite));
        assert_eq!(risk_for("push"), Some(GitRisk::RemoteWrite));
        assert_eq!(risk_for("unknown"), None);
    }

    #[test]
    fn malformed_pathspec_is_rejected() {
        let error = parse_pathspec(&json!({"pathspec": ["ok", 3]})).unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}
