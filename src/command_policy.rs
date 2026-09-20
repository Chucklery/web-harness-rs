use std::path::{Component, Path};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CommandPolicyError {
    #[error("executable path traversal is not allowed")]
    Traversal,
    #[error("host-control executable is not allowed: {0}")]
    HostControl(String),
    #[error("inline evaluation flags are not allowed with {0}; pass a script file inside the workspace instead")]
    InlineEvaluation(String),
}

/// Inline evaluation flags, checked by name rather than per interpreter so a
/// new interpreter cannot quietly reintroduce the surface.
///
/// `-e` style flags are deliberately absent: they carry the script in argument
/// position (`bash -e script.sh`, `sed -e 's/a/b/'`), so rejecting them would
/// break ordinary commands without closing the inline-script surface.
const INLINE_FLAGS: &[&str] = &["-c", "--command"];

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

    if is_known_shell(path) {
        // Reject the first inline flag. Any payload after it is inert because
        // nothing is executed; this also stops the error message from echoing
        // the script the caller tried to run.
        if let Some(flag) = argv[1..].iter().find(|arg| is_inline_flag(arg)) {
            return Err(CommandPolicyError::InlineEvaluation(flag.clone()));
        }
    }
    Ok(())
}

/// Shells and interpreters whose inline-evaluation flag turns `exec` into an
/// unrestricted command surface.
///
/// `exec` has no shell-string mode and its sandbox profile grants network
/// access, so one inline flag would bypass the argv model entirely. This is a
/// policy filter, not a security boundary: `PATH` may still contain an alias or
/// a wrapper such as `env` or `xargs`, and macOS Seatbelt (`exec` on any other
/// platform) remains the boundary.
fn is_known_shell(executable: &Path) -> bool {
    const SHELLS: &[&str] = &[
        "ash", "bash", "busybox", "csh", "dash", "elvish", "fish", "ksh", "lua", "node", "perl",
        "php", "pwsh", "python", "python2", "python3", "ruby", "sh", "tcsh", "zsh",
    ];
    let Some(raw) = executable.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let raw = raw.to_ascii_lowercase();
    let stem = raw
        .strip_suffix(".exe")
        .unwrap_or(&raw)
        .trim_end_matches(|character: char| character.is_ascii_digit())
        .trim_end_matches('.');
    // `python3.13` collapses to `python`; digits only ever appear in trailing
    // version components, so `busybox` and `pwsh` are still matched by name.
    SHELLS.contains(&stem)
}

fn is_inline_flag(argument: &str) -> bool {
    INLINE_FLAGS.contains(&argument) || INLINE_FLAGS.contains(&&*argument.to_ascii_lowercase())
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
    fn rejects_inline_evaluation_for_known_interpreters() {
        assert!(matches!(
            validate_argv(&["sh".into(), "-c".into(), "pwd".into()]),
            Err(CommandPolicyError::InlineEvaluation(_))
        ));
        assert!(matches!(
            validate_argv(&["/bin/sh".into(), "-c".into(), "pwd".into()]),
            Err(CommandPolicyError::InlineEvaluation(_))
        ));
        // The interpreter name is recognized through versions and suffixes.
        assert!(matches!(
            validate_argv(&["python3.13".into(), "-c".into(), "print(1)".into()]),
            Err(CommandPolicyError::InlineEvaluation(_))
        ));
        assert!(matches!(
            validate_argv(&["ruby".into(), "-c".into(), "puts 1".into()]),
            Err(CommandPolicyError::InlineEvaluation(_))
        ));
        assert!(matches!(
            validate_argv(&["bash".into(), "--command".into(), "pwd".into()]),
            Err(CommandPolicyError::InlineEvaluation(_))
        ));
        // A wrapper hides the interpreter name and is not caught: this is a
        // policy filter, and the sandbox stays the boundary.
        validate_argv(&["/usr/bin/env".into(), "sh".into(), "-c".into(), "pwd".into()]).unwrap();
    }

    #[test]
    fn inline_flags_that_take_a_file_argument_stay_usable() {
        // `sh -e script.sh` and `sed -e ...` are arguments, not inline scripts.
        validate_argv(&["sh".into(), "-e".into(), "script.sh".into()]).unwrap();
        validate_argv(&["sed".into(), "-e".into(), "s/a/b/".into()]).unwrap();
    }

    #[test]
    fn allows_scripts_and_arguments_after_the_script_name() {
        validate_argv(&["sh".into(), "script.sh".into()]).unwrap();
        validate_argv(&["sh".into(), "--".into()]).unwrap();
        // The inline flag is rejected even when it appears after `--`.
        assert!(matches!(
            validate_argv(&["python3".into(), "--".into(), "-c".into()]),
            Err(CommandPolicyError::InlineEvaluation(_))
        ));
    }

    #[test]
    fn does_not_police_arguments_of_other_executables() {
        validate_argv(&["cargo".into(), "test".into(), "-c".into()]).unwrap();
        validate_argv(&["/usr/bin/env".into(), "cargo".into(), "test".into()]).unwrap();
    }

    #[test]
    fn allows_normal_developer_tools() {
        validate_argv(&["cargo".into(), "test".into()]).unwrap();
        validate_argv(&["/usr/bin/git".into(), "status".into()]).unwrap();
        validate_argv(&["rm".into(), "-f".into(), "target/file".into()]).unwrap();
    }
}
