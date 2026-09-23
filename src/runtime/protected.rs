use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeToolError};
use crate::permission::{Capability, ExecAuthorization};
use crate::sandbox::NetworkPolicy;
use serde_json::{json, Value};

pub fn authorization(argv: Vec<String>, protected_read: bool) -> ExecAuthorization {
    ExecAuthorization {
        capability: Capability::WorkspaceSensitiveRead,
        argv,
        cwd: Some(".".into()),
        background: false,
        network: NetworkPolicy::Deny,
        expected_head: None,
        stdin: None,
        protected_read,
    }
}

pub fn authorize_or_request(
    context: &mut ExecutionContext<'_>,
    approval_id: Option<&str>,
    authorization: &ExecAuthorization,
    summary: &str,
    protected_paths: &[String],
) -> Result<Option<Value>, RuntimeToolError> {
    if let Some(approval_id) = approval_id {
        context
            .permissions()
            .consume_exec(approval_id, authorization)
            .map_err(permission_error)?;
        return Ok(None);
    }

    let approval = context
        .permissions()
        .request_action(
            authorization,
            summary.to_string(),
            "Sensitive paths require explicit one-time user approval".into(),
        )
        .map_err(permission_error)?;
    Ok(Some(json!({
        "status": "approval_required",
        "approval": approval,
        "capability": Capability::WorkspaceSensitiveRead.as_str(),
        "protected_paths": protected_paths
    })))
}

fn permission_error(error: crate::permission::PermissionError) -> RuntimeToolError {
    RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
}
