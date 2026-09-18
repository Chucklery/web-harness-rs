use std::path::{Component, Path};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommandPolicyError {
    #[error("executable path traversal is not allowed")]
    Traversal,
    #[error("host-control executable is not allowed: {0}")]
    HostControl(String),
}

pub fn validate_argv(argv: &[String]) -> Result<(), CommandPolicyError> {
    let executable = argv.first().map(String::as_str).unwrap_or_default();
    let path = Path::new(executable);
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(CommandPolicyError::Traversal);
    }

    let basename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(executable)
        .to_ascii_lowercase();
    if matches!(
        basename.as_str(),
        "shutdown" | "reboot" | "halt" | "poweroff" | "diskutil" | "fdisk" | "mkfs" | "sudo" | "su"
    ) {
        return Err(CommandPolicyError::HostControl(basename));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_executable_parent_traversal() {
        assert_eq!(
            validate_argv(&["../tool".into()]).unwrap_err(),
            CommandPolicyError::Traversal
        );
    }

    #[test]
    fn rejects_host_control_tools() {
        assert!(matches!(
            validate_argv(&["/usr/sbin/diskutil".into(), "list".into()]),
            Err(CommandPolicyError::HostControl(_))
        ));
        assert!(matches!(
            validate_argv(&["sudo".into(), "echo".into()]),
            Err(CommandPolicyError::HostControl(_))
        ));
    }

    #[test]
    fn allows_normal_developer_tools() {
        validate_argv(&["cargo".into(), "test".into()]).unwrap();
        validate_argv(&["/usr/bin/git".into(), "status".into()]).unwrap();
        validate_argv(&["rm".into(), "-f".into(), "target/file".into()]).unwrap();
    }
}
