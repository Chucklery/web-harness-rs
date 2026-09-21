use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::command_policy::{self, CommandRisk};
use crate::exec;
use crate::permission::{Capability, ExecAuthorization};
use crate::sandbox::NetworkPolicy;
use serde_json::{json, Value};

const MAX_ARGV_ITEMS: usize = 64;
const MAX_TIMEOUT_MS: u64 = 600_000;
const MAX_STDIN_BYTES: usize = 64 * 1024;

pub struct ExecRuntime;

impl RuntimeTool for ExecRuntime {
    fn name(&self) -> &'static str {
        "exec"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        let values = arguments
            .get("argv")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "arguments.argv is required",
                )
            })?;
        if values.is_empty() || values.len() > MAX_ARGV_ITEMS {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "argv must contain 1..=64 items",
            ));
        }
        let argv = values
            .iter()
            .map(|value| {
                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                    RuntimeToolError::new(
                        RuntimeErrorKind::InvalidArguments,
                        "argv items must be strings",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let cwd = arguments.get("cwd").and_then(Value::as_str);
        let background = arguments
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let timeout_ms = arguments.get("timeout_ms").and_then(Value::as_u64);
        if timeout_ms.is_some_and(|value| value == 0 || value > MAX_TIMEOUT_MS) {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "timeout_ms must be 1..=600000",
            ));
        }
        let stdin = arguments
            .get("stdin")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if stdin
            .as_ref()
            .is_some_and(|value| value.len() > MAX_STDIN_BYTES)
        {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "stdin must be at most 65536 UTF-8 bytes",
            ));
        }

        let network = match arguments
            .get("network")
            .and_then(Value::as_str)
            .unwrap_or("deny")
        {
            "deny" => NetworkPolicy::Deny,
            "outbound" => NetworkPolicy::Outbound,
            _ => {
                return Err(RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "network must be deny or outbound",
                ))
            }
        };

        let sandbox = context.sandbox();
        if network == NetworkPolicy::Outbound && !crate::sandbox::can_upgrade_network() {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::Permission,
                "outbound network is unavailable without a native sandbox backend",
            ));
        }

        let capability = match (command_policy::command_risk(&argv), network) {
            (Some(CommandRisk::GitLocalWrite), _) => Capability::GitLocalWrite,
            (Some(CommandRisk::GitRemoteWrite), _) => Capability::GitRemoteWrite,
            (None, NetworkPolicy::Outbound) => Capability::NetworkOutbound,
            (None, NetworkPolicy::Deny) => Capability::ProcessExecute,
        };
        let authorization = ExecAuthorization {
            capability,
            argv: argv.clone(),
            cwd: cwd.map(ToOwned::to_owned),
            background,
            network,
            expected_head: None,
            stdin: stdin.clone(),
        };
        if capability.requires_approval()
            && (capability != Capability::ProcessExecute || !sandbox.enforced())
        {
            if let Some(approval_id) = arguments.get("approval_id").and_then(Value::as_str) {
                context
                    .permissions()
                    .consume_exec(approval_id, &authorization)
                    .map_err(|error| {
                        RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
                    })?;
            } else {
                let (summary, reason) = match capability {
                    Capability::GitLocalWrite => (
                        "Run Git through exec".to_string(),
                        "Direct Git execution may change the local repository and requires explicit one-time approval".to_string(),
                    ),
                    Capability::GitRemoteWrite => (
                        "Run Git push through exec".to_string(),
                        "Direct Git push may change a remote repository and requires explicit one-time approval".to_string(),
                    ),
                    Capability::NetworkOutbound => (
                        format!("Run {} with outbound network", argv.first().map(String::as_str).unwrap_or("?")),
                        "Outbound network is denied by default and requires explicit one-time approval".to_string(),
                    ),
                    _ => (
                        format!("Run {}", argv.first().map(String::as_str).unwrap_or("?")),
                        "OS sandbox enforcement is not enabled; explicit approval is required".to_string(),
                    ),
                };
                let approval =
                    context
                        .permissions()
                        .request_action(&authorization, summary, reason);
                return Ok(json!({
                    "status": "approval_required",
                    "approval": approval,
                    "capability": capability.as_str()
                }));
            }
        }

        let workspace = context.workspace().clone();
        let value = if background {
            serde_json::to_value(
                exec::background(
                    context.jobs(),
                    &workspace,
                    &argv,
                    cwd,
                    sandbox.enforced(),
                    network,
                    stdin.as_deref().map(str::as_bytes),
                )
                .map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
                })?,
            )
        } else {
            serde_json::to_value(
                exec::foreground(
                    context.jobs(),
                    &workspace,
                    &argv,
                    cwd,
                    timeout_ms,
                    sandbox.enforced(),
                    network,
                    stdin.as_deref().map(str::as_bytes),
                )
                .map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
                })?,
            )
        }
        .map_err(|error| RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string()))?;

        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;

    #[test]
    fn validates_exec_shape_before_execution() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let error = ExecRuntime
            .call(&mut context, &json!({"argv": []}))
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn direct_git_uses_git_approval_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let local = ExecRuntime
            .call(&mut context, &json!({"argv": ["git", "status"]}))
            .unwrap();
        assert_eq!(local["status"], "approval_required");
        assert_eq!(local["capability"], "git.local.write");

        let remote = ExecRuntime
            .call(&mut context, &json!({"argv": ["git", "push"]}))
            .unwrap();
        assert_eq!(remote["status"], "approval_required");
        assert_eq!(remote["capability"], "git.remote.write");
    }

    #[test]
    fn rejects_unknown_network_policy() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let error = ExecRuntime
            .call(
                &mut context,
                &json!({"argv": ["cargo", "check"], "network": "internet"}),
            )
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}
