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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRisk {
    GitLocalWrite,
    GitRemoteWrite,
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

/// Classifies direct Git execution before spawn so it shares the structured
/// Git approval capabilities. Wrappers are intentionally not unwrapped here:
/// this is a clear policy decision, while the OS sandbox remains the boundary.
pub fn command_risk(argv: &[String]) -> Option<CommandRisk> {
    let executable = argv.first()?;
    let basename = Path::new(executable)
        .file_name()
        .and_then(|value| value.to_str())?;
    let basename = basename
        .strip_suffix(".exe")
        .unwrap_or(basename)
        .to_ascii_lowercase();
    if basename != "git" {
        return None;
    }

    match git_subcommand(argv) {
        Some("push") => Some(CommandRisk::GitRemoteWrite),
        Some(subcommand) if is_git_builtin(subcommand) => {
            // Even nominally read-only Git commands accept options and config
            // with side effects, so all recognized built-ins require local
            // repository approval.
            Some(CommandRisk::GitLocalWrite)
        }
        // An unknown subcommand may be a configured `!` alias that executes
        // arbitrary commands (including `git push`). Require the stronger
        // capability because static argv inspection cannot resolve Git config.
        _ => Some(CommandRisk::GitRemoteWrite),
    }
}

fn is_git_builtin(subcommand: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "add",
        "am",
        "annotate",
        "apply",
        "archive",
        "bisect",
        "blame",
        "branch",
        "bundle",
        "cat-file",
        "check-attr",
        "check-ignore",
        "check-mailmap",
        "check-ref-format",
        "checkout",
        "cherry",
        "cherry-pick",
        "citool",
        "clean",
        "clone",
        "commit",
        "config",
        "count-objects",
        "credential",
        "describe",
        "diff",
        "difftool",
        "fast-export",
        "fetch",
        "filter-branch",
        "for-each-ref",
        "format-patch",
        "fsck",
        "gc",
        "get-tar-commit-id",
        "grep",
        "gui",
        "hash-object",
        "help",
        "init",
        "instaweb",
        "log",
        "maintenance",
        "merge",
        "mergetool",
        "mktag",
        "mktree",
        "mv",
        "name-rev",
        "notes",
        "pack-objects",
        "pack-redundant",
        "pack-refs",
        "patch-id",
        "prune",
        "pull",
        "push",
        "range-diff",
        "read-tree",
        "rebase",
        "reflog",
        "remote",
        "repack",
        "replace",
        "request-pull",
        "rerere",
        "reset",
        "restore",
        "rev-list",
        "rev-parse",
        "rm",
        "scalar",
        "send-pack",
        "shortlog",
        "show",
        "show-branch",
        "sparse-checkout",
        "stash",
        "status",
        "submodule",
        "switch",
        "tag",
        "worktree",
        "write-tree",
    ];
    BUILTINS.contains(&subcommand)
}

/// Conservatively classifies recognizable Git mutation tokens in a one-shot script.
/// This only selects an extra approval prompt; sandboxing remains the boundary.
pub fn script_git_risk(script: &str) -> Option<CommandRisk> {
    let mut mentions_git = false;
    let mut mentioned_subcommands = Vec::new();

    for word in
        script.split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
    {
        if word.is_empty() {
            continue;
        }
        let word = word.to_ascii_lowercase();
        mentions_git |= word == "git";
        mentioned_subcommands.push(word);
    }

    if !mentions_git {
        return None;
    }
    if mentioned_subcommands.iter().any(|word| word == "push") {
        return Some(CommandRisk::GitRemoteWrite);
    }
    const LOCAL_MUTATIONS: &[&str] = &[
        "add",
        "am",
        "branch",
        "checkout",
        "cherry-pick",
        "clean",
        "clone",
        "commit",
        "config",
        "fetch",
        "init",
        "merge",
        "mv",
        "rebase",
        "reset",
        "restore",
        "rm",
        "stash",
        "switch",
        "tag",
        "worktree",
    ];
    mentioned_subcommands
        .iter()
        .any(|word| LOCAL_MUTATIONS.contains(&word.as_str()))
        .then_some(CommandRisk::GitLocalWrite)
}

fn git_subcommand(argv: &[String]) -> Option<&str> {
    let mut index = 1;
    while index < argv.len() {
        let argument = argv[index].as_str();
        if argument == "--" {
            return argv.get(index + 1).map(String::as_str);
        }
        if matches!(
            argument,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            index += 2;
            continue;
        }
        if argument.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(argument);
    }
    None
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
        validate_argv(&[
            "/usr/bin/env".into(),
            "sh".into(),
            "-c".into(),
            "pwd".into(),
        ])
        .unwrap();
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

    #[test]
    fn classifies_direct_git_with_global_options() {
        assert_eq!(
            command_risk(&["/usr/bin/git".into(), "status".into()]),
            Some(CommandRisk::GitLocalWrite)
        );
        assert_eq!(
            command_risk(&["git".into(), "-C".into(), "nested".into(), "push".into()]),
            Some(CommandRisk::GitRemoteWrite)
        );
        assert_eq!(
            command_risk(&["git".into(), "deploy".into()]),
            Some(CommandRisk::GitRemoteWrite)
        );
        assert_eq!(
            command_risk(&[
                "git".into(),
                "-c".into(),
                "alias.deploy=!git push".into(),
                "deploy".into(),
            ]),
            Some(CommandRisk::GitRemoteWrite)
        );
        assert_eq!(command_risk(&["cargo".into(), "test".into()]), None);
    }

    #[test]
    fn script_git_mutations_are_classified_conservatively() {
        assert_eq!(
            script_git_risk("git push origin main"),
            Some(CommandRisk::GitRemoteWrite)
        );
        assert_eq!(
            script_git_risk("/usr/bin/git -C repo push"),
            Some(CommandRisk::GitRemoteWrite)
        );
        assert_eq!(
            script_git_risk("& git.exe add src/main.rs"),
            Some(CommandRisk::GitLocalWrite)
        );
        assert_eq!(script_git_risk("git status"), None);
        assert_eq!(script_git_risk("printf push"), None);
    }
}
