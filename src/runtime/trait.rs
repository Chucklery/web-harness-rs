use crate::workspace::Workspace;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorKind {
    InvalidArguments,
    Workspace,
    LimitExceeded,
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

    fn call(&self, workspace: &Workspace, arguments: &Value) -> Result<Value, RuntimeToolError>;
}
