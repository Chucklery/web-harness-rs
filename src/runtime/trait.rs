use super::context::ExecutionContext;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorKind {
    InvalidArguments,
    Workspace,
    LimitExceeded,
    Execution,
    Permission,
    Conflict,
    /// A runtime dependency the tool shells out to is missing or unusable.
    ///
    /// Kept distinct from [`RuntimeErrorKind::Execution`] because the caller
    /// cannot fix it by changing the request: retrying is pointless, and an
    /// agent that cannot tell the two apart will loop on an unfixable call.
    Dependency,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct RuntimeToolError {
    kind: RuntimeErrorKind,
    message: String,
}

impl RuntimeToolError {
    pub fn new(kind: RuntimeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> RuntimeErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub trait RuntimeTool {
    fn name(&self) -> &'static str;

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError>;
}
