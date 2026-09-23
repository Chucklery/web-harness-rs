use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::command_policy::{self, CommandRisk};
use crate::exec;
use crate::path_policy;
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
        let script = arguments
            .get("script")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let argv = match (arguments.get("argv"), script.as_deref()) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "provide exactly one of argv or script",
                ))
            }
            (Some(value), None) => {
                let values = value.as_array().ok_or_else(|| {
                    RuntimeToolError::new(
                        RuntimeErrorKind::InvalidArguments,
                        "arguments.argv must be an array",
                    )
                })?;
                if values.is_empty() || values.len() > MAX_ARGV_ITEMS {
                    return Err(RuntimeToolError::new(
                        RuntimeErrorKind::InvalidArguments,
                        "argv must contain 1..=64 items",
                    ));
                }
                values
                    .iter()
                    .map(|value| {
                        value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                            RuntimeToolError::new(
                                RuntimeErrorKind::InvalidArguments,
                                "argv items must be strings",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            (None, Some(_)) => {
                let shell = arguments
                    .get("shell")
                    .and_then(Value::as_str)
                    .unwrap_or("sh");
                validate_script_shell(shell)?;
                vec![shell.to_string()]
            }
        };

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
        let explicit_stdin = arguments
            .get("stdin")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if script.is_some() && explicit_stdin.is_some() {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "script and stdin cannot be provided together",
            ));
        }
        let stdin = script.clone().or(explicit_stdin);
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

        let command_capability = match command_policy::command_risk(&argv) {
            Some(CommandRisk::GitLocalWrite) => Some(Capability::GitLocalWrite),
            Some(CommandRisk::GitRemoteWrite) => Some(Capability::GitRemoteWrite),
            None if script.is_some() || !sandbox.enforced() => Some(Capability::ProcessExecute),
            None => None,
        };
        let script_git_risk = script.as_deref().and_then(command_policy::script_git_risk);
        let protected_read = argv.iter().skip(1).any(|argument| {
            path_policy::is_protected_workspace_path(context.workspace(), argument)
        });
        let authorization = ExecAuthorization {
            capability: command_capability.unwrap_or(Capability::ProcessExecute),
            argv: argv.clone(),
            cwd: cwd.map(ToOwned::to_owned),
            background,
            network,
            expected_head: None,
            stdin: stdin.clone(),
            protected_read,
        };
        let script_mode = script.is_some();
        let needs_command_approval =
            command_capability.is_some() || protected_read || script_mode || !sandbox.enforced();
        let mut approvals = Vec::with_capacity(3);
        if needs_command_approval {
            let capability = command_capability.unwrap_or(Capability::ProcessExecute);
            let mut command_authorization = authorization.clone();
            command_authorization.capability = capability;
            if let Some(approval_id) = arguments.get("approval_id").and_then(Value::as_str) {
                context
                    .permissions()
                    .validate_exec(approval_id, &command_authorization)
                    .map_err(|error| {
                        RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
                    })?;
                approvals.push((approval_id.to_string(), command_authorization));
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
                    Capability::ProcessExecute if script_mode => (
                        "Run an approved workspace script".to_string(),
                        "Script execution requires explicit one-time approval".to_string(),
                    ),
                    Capability::ProcessExecute if protected_read => (
                        "Read a protected workspace path through exec".to_string(),
                        "Protected paths require explicit one-time approval".to_string(),
                    ),
                    Capability::ProcessExecute => (
                        format!("Run {}", argv.first().map(String::as_str).unwrap_or("?")),
                        "OS sandbox enforcement is not enabled; explicit approval is required".to_string(),
                    ),
                    _ => unreachable!("only command capabilities are handled here"),
                };
                return request_approval(
                    context,
                    &command_authorization,
                    "approval_id",
                    capability,
                    summary,
                    reason,
                );
            }
        }

        if let Some(git_risk) = script_git_risk {
            let mut git_authorization = authorization.clone();
            let capability = match git_risk {
                CommandRisk::GitLocalWrite => Capability::GitLocalWrite,
                CommandRisk::GitRemoteWrite => Capability::GitRemoteWrite,
            };
            git_authorization.capability = capability;
            if let Some(approval_id) = arguments.get("git_approval_id").and_then(Value::as_str) {
                context
                    .permissions()
                    .validate_exec(approval_id, &git_authorization)
                    .map_err(|error| {
                        RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
                    })?;
                approvals.push((approval_id.to_string(), git_authorization));
            } else {
                return request_approval(
                    context,
                    &git_authorization,
                    "git_approval_id",
                    capability,
                    match git_risk {
                        CommandRisk::GitLocalWrite => {
                            "Allow this script to mutate the local Git repository".to_string()
                        }
                        CommandRisk::GitRemoteWrite => {
                            "Allow this script to push to a Git remote".to_string()
                        }
                    },
                    match git_risk {
                        CommandRisk::GitLocalWrite => "A Git mutation in script mode requires a separate git.local.write approval".to_string(),
                        CommandRisk::GitRemoteWrite => "A Git push in script mode requires a separate git.remote.write approval".to_string(),
                    },
                );
            }
        }

        if network == NetworkPolicy::Outbound {
            let mut network_authorization = authorization.clone();
            network_authorization.capability = Capability::NetworkOutbound;
            if let Some(approval_id) = arguments.get("network_approval_id").and_then(Value::as_str)
            {
                context
                    .permissions()
                    .validate_exec(approval_id, &network_authorization)
                    .map_err(|error| {
                        RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
                    })?;
                approvals.push((approval_id.to_string(), network_authorization));
            } else {
                return request_approval(
                    context,
                    &network_authorization,
                    "network_approval_id",
                    Capability::NetworkOutbound,
                    format!(
                        "Run {} with outbound network",
                        argv.first().map(String::as_str).unwrap_or("?")
                    ),
                    "Outbound network is denied by default and requires explicit one-time approval"
                        .to_string(),
                );
            }
        }
        if !approvals.is_empty() {
            context
                .permissions()
                .consume_execs(&approvals)
                .map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
                })?;
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

fn request_approval(
    context: &mut ExecutionContext<'_>,
    authorization: &ExecAuthorization,
    argument_name: &str,
    capability: Capability,
    summary: String,
    reason: String,
) -> Result<Value, RuntimeToolError> {
    let approval = context
        .permissions()
        .request_action(authorization, summary, reason)
        .map_err(permission_error)?;
    Ok(json!({
        "status": "approval_required",
        "approval": approval,
        "capability": capability.as_str(),
        "approval_argument": argument_name
    }))
}

fn permission_error(error: crate::permission::PermissionError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
}

fn validate_script_shell(shell: &str) -> Result<(), RuntimeToolError> {
    let allowed = if cfg!(windows) {
        matches!(shell, "powershell" | "pwsh")
    } else {
        matches!(shell, "sh" | "bash")
    };
    if !allowed {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "shell must be sh or bash on Unix, or powershell or pwsh on Windows",
        ));
    }
    Ok(())
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
    fn git_push_with_outbound_network_requires_two_approvals() {
        if !crate::sandbox::can_upgrade_network() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let request = json!({"argv":["git","push"],"network":"outbound"});

        let git_approval = ExecRuntime.call(&mut context, &request).unwrap();
        assert_eq!(git_approval["capability"], "git.remote.write");
        assert_eq!(git_approval["approval_argument"], "approval_id");
        let git_ticket = git_approval["approval"]["id"].as_str().unwrap().to_string();
        context.permissions().approve(&git_ticket).unwrap();

        let mut retry = request.as_object().unwrap().clone();
        retry.insert("approval_id".into(), json!(git_ticket));
        let retry = Value::Object(retry);
        let network_approval = ExecRuntime.call(&mut context, &retry).unwrap();
        assert_eq!(network_approval["capability"], "network.outbound");
        assert_eq!(network_approval["approval_argument"], "network_approval_id");
        let network_ticket = network_approval["approval"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        context.permissions().approve(&network_ticket).unwrap();

        let mut retry = retry.as_object().unwrap().clone();
        retry.insert("network_approval_id".into(), json!(network_ticket));
        let result = ExecRuntime
            .call(&mut context, &Value::Object(retry))
            .unwrap();
        assert_eq!(result["exit_code"], 128);
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

    #[test]
    fn script_mode_requires_explicit_approval() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let result = ExecRuntime
            .call(&mut context, &json!({"script": "printf script"}))
            .unwrap();
        assert_eq!(result["status"], "approval_required");
        assert_eq!(result["capability"], "process.execute");
    }

    #[test]
    fn script_git_push_requires_a_separate_remote_approval() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let request = json!({"script":"printf 'git push policy probe\\n'"});

        let process_approval = ExecRuntime.call(&mut context, &request).unwrap();
        assert_eq!(process_approval["capability"], "process.execute");
        let process_ticket = process_approval["approval"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        context.permissions().approve(&process_ticket).unwrap();

        let mut retry = request.as_object().unwrap().clone();
        retry.insert("approval_id".into(), json!(process_ticket));
        let retry = Value::Object(retry);
        let git_approval = ExecRuntime.call(&mut context, &retry).unwrap();
        assert_eq!(git_approval["capability"], "git.remote.write");
        assert_eq!(git_approval["approval_argument"], "git_approval_id");
        let git_ticket = git_approval["approval"]["id"].as_str().unwrap().to_string();

        let network_authorization = ExecAuthorization {
            capability: Capability::NetworkOutbound,
            argv: vec!["sh".into()],
            cwd: None,
            background: false,
            network: NetworkPolicy::Deny,
            expected_head: None,
            stdin: Some("printf 'git push policy probe\\n'".into()),
            protected_read: false,
        };
        let network_ticket = context
            .permissions()
            .request_action(&network_authorization, "network only".into(), "test".into())
            .unwrap();
        context.permissions().approve(&network_ticket.id).unwrap();
        let mut mismatched_retry = retry.as_object().unwrap().clone();
        mismatched_retry.insert("git_approval_id".into(), json!(network_ticket.id));
        let mismatch = ExecRuntime
            .call(&mut context, &Value::Object(mismatched_retry))
            .unwrap_err();
        assert_eq!(mismatch.kind(), RuntimeErrorKind::Permission);
        context.permissions().approve(&git_ticket).unwrap();

        let mut retry = retry.as_object().unwrap().clone();
        retry.insert("git_approval_id".into(), json!(git_ticket));
        let result = ExecRuntime
            .call(&mut context, &Value::Object(retry))
            .unwrap();
        assert_eq!(result["exit_code"], 0);
        assert!(result["stdout_tail"]
            .as_str()
            .unwrap()
            .contains("git push policy probe"));
    }

    #[test]
    fn script_git_local_mutation_requires_a_separate_local_approval() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let request = json!({"script":"printf 'git add policy probe\\n'"});

        let process_approval = ExecRuntime.call(&mut context, &request).unwrap();
        assert_eq!(process_approval["capability"], "process.execute");
        let process_ticket = process_approval["approval"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        context.permissions().approve(&process_ticket).unwrap();

        let mut retry = request.as_object().unwrap().clone();
        retry.insert("approval_id".into(), json!(process_ticket));
        let retry = Value::Object(retry);
        let git_approval = ExecRuntime.call(&mut context, &retry).unwrap();
        assert_eq!(git_approval["capability"], "git.local.write");
        assert_eq!(git_approval["approval_argument"], "git_approval_id");
        let git_ticket = git_approval["approval"]["id"].as_str().unwrap().to_string();
        context.permissions().approve(&git_ticket).unwrap();

        let mut mismatched_retry = retry.as_object().unwrap().clone();
        mismatched_retry.insert("git_approval_id".into(), json!(process_ticket));
        assert_eq!(
            ExecRuntime
                .call(&mut context, &Value::Object(mismatched_retry))
                .unwrap_err()
                .kind(),
            RuntimeErrorKind::Permission
        );

        let mut retry = retry.as_object().unwrap().clone();
        retry.insert("git_approval_id".into(), json!(git_ticket));
        let result = ExecRuntime
            .call(&mut context, &Value::Object(retry))
            .unwrap();
        assert_eq!(result["exit_code"], 0);
        assert!(result["stdout_tail"]
            .as_str()
            .unwrap()
            .contains("git add policy probe"));
    }

    #[test]
    fn direct_exec_reading_existing_protected_path_requires_approval() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=secret\n").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let result = ExecRuntime
            .call(&mut context, &json!({"argv": ["cat", ".env"]}))
            .unwrap();
        assert_eq!(result["status"], "approval_required");
        assert_eq!(result["capability"], "process.execute");
    }

    #[cfg(unix)]
    #[test]
    fn direct_exec_reading_symlink_to_protected_path_requires_approval() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=secret\n").unwrap();
        std::os::unix::fs::symlink(".env", dir.path().join("config.txt")).unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        let result = ExecRuntime
            .call(&mut context, &json!({"argv": ["cat", "config.txt"]}))
            .unwrap();
        assert_eq!(result["status"], "approval_required");
        assert_eq!(result["capability"], "process.execute");
    }
}
