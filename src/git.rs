use crate::command_output;
use crate::env;
use crate::path_policy;
use crate::redact;
use crate::workspace::{Workspace, WorkspaceError};
use serde::Serialize;
use std::process::{Command, Stdio};
use thiserror::Error;

const MAX_OUTPUT: usize = 512 * 1024;
const MAX_FILE_CAPTURE: usize = 8 * 1024 * 1024;
const MAX_FILE_PAGE: usize = 256 * 1024;
const MAX_FILE_START_LINE: usize = 1_000_000;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git is not available")]
    Unavailable,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("git result limit exceeded: {0}")]
    Limit(String),
    #[error("git command failed: {0}")]
    Failed(String),
    #[error("invalid git request: {0}")]
    Invalid(String),
}

#[derive(Debug, Serialize)]
pub struct GitResult {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    #[serde(skip)]
    stdout_truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct GitDiffPage {
    pub hunks: Vec<String>,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct GitFilePage {
    pub revision: String,
    pub path: String,
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
    pub next_start_line: Option<usize>,
    pub truncated: bool,
    pub source_truncated: bool,
}

pub fn status(workspace: &Workspace) -> Result<GitResult, GitError> {
    run(workspace, &["status", "--short", "--branch"])
}

pub fn head(workspace: &Workspace) -> Result<String, GitError> {
    Ok(run(workspace, &["rev-parse", "HEAD"])?
        .stdout
        .trim()
        .to_string())
}

pub fn diff(
    workspace: &Workspace,
    staged: bool,
    pathspec: &[String],
) -> Result<GitResult, GitError> {
    validate_pathspec(workspace, pathspec, false)?;
    run_owned(workspace, &diff_args(staged, pathspec))
}

/// Return changed protected paths without reading or returning their contents.
pub fn protected_diff_paths(
    workspace: &Workspace,
    staged: bool,
    pathspec: &[String],
) -> Result<Vec<String>, GitError> {
    validate_pathspec(workspace, pathspec, false)?;
    let mut args = vec![
        "diff".to_string(),
        "--name-only".to_string(),
        "--no-color".to_string(),
        "-z".to_string(),
    ];
    if staged {
        args.push("--cached".into());
    }
    if !pathspec.is_empty() {
        args.push("--".into());
        args.extend(pathspec.iter().cloned());
    }
    let result = run_owned(workspace, &args)?;
    if result.stdout_truncated {
        return Err(GitError::Limit(
            "changed Git path scan reached its output limit".into(),
        ));
    }
    Ok(result
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty() && path_policy::is_protected(path))
        .map(ToOwned::to_owned)
        .collect())
}

/// Canonical command payload used both for diff execution and approval binding.
pub fn diff_authorization_argv(staged: bool, pathspec: &[String]) -> Vec<String> {
    let mut argv = vec!["git".to_string()];
    argv.extend(diff_args(staged, pathspec));
    argv
}

fn diff_args(staged: bool, pathspec: &[String]) -> Vec<String> {
    let mut args = vec!["diff".to_string(), "--no-color".to_string()];
    if staged {
        args.push("--cached".into());
    }
    if !pathspec.is_empty() {
        args.push("--".into());
        args.extend(pathspec.iter().cloned());
    }
    args
}

pub fn diff_page(
    workspace: &Workspace,
    staged: bool,
    pathspec: &[String],
    offset: u64,
    limit: u64,
) -> Result<GitDiffPage, GitError> {
    let result = diff(workspace, staged, pathspec)?;
    let chunks = split_diff_hunks(&result.stdout);
    let offset = (offset as usize).min(chunks.len());
    let limit = limit.clamp(1, 32) as usize;
    let end = offset.saturating_add(limit).min(chunks.len());
    let next_offset = if result.truncated || end == chunks.len() {
        None
    } else {
        Some(end)
    };
    Ok(GitDiffPage {
        hunks: chunks[offset..end].to_vec(),
        offset,
        next_offset,
        truncated: result.truncated || next_offset.is_some(),
    })
}

pub fn log(workspace: &Workspace, limit: u64) -> Result<GitResult, GitError> {
    let limit = limit.clamp(1, 100).to_string();
    run(
        workspace,
        &["log", "--oneline", "--decorate", "--no-color", "-n", &limit],
    )
}

pub fn show(workspace: &Workspace, revision: &str) -> Result<GitResult, GitError> {
    validate_ref_name(revision)?;
    run(
        workspace,
        &["show", "--stat", "--oneline", "--no-color", revision],
    )
}

pub fn show_file_page(
    workspace: &Workspace,
    revision: &str,
    path: &str,
    start_line: usize,
    max_bytes: usize,
) -> Result<GitFilePage, GitError> {
    validate_ref_name(revision)?;
    if !(1..=MAX_FILE_START_LINE).contains(&start_line) {
        return Err(GitError::Invalid(format!(
            "start_line must be 1..={MAX_FILE_START_LINE}"
        )));
    }
    if !(1..=MAX_FILE_PAGE).contains(&max_bytes) {
        return Err(GitError::Invalid(format!(
            "max_bytes must be 1..={MAX_FILE_PAGE}"
        )));
    }
    workspace
        .resolve_for_write(path)
        .map_err(GitError::Workspace)?;
    let result = run_owned_with_limit(
        workspace,
        &["show".into(), format!("{revision}:{path}")],
        MAX_FILE_CAPTURE,
    )?;
    let mut line_number = 0usize;
    let mut output = String::with_capacity(max_bytes.min(64 * 1024));
    let mut end_line = start_line.saturating_sub(1);
    let mut next_start_line = None;
    for line in result.stdout.split_inclusive('\n') {
        line_number += 1;
        if line_number < start_line {
            continue;
        }
        if line.len() > max_bytes {
            return Err(GitError::Limit(format!(
                "line {line_number} exceeds the show_file page limit"
            )));
        }
        if output.len().saturating_add(line.len()) > max_bytes {
            let line_is_complete = line.ends_with('\n') || !result.truncated;
            if line_is_complete {
                next_start_line = Some(line_number);
            }
            break;
        }
        output.push_str(line);
        end_line = line_number;
    }
    let source_truncated = result.stdout_truncated;
    let truncated = source_truncated || next_start_line.is_some();
    Ok(GitFilePage {
        revision: revision.to_string(),
        path: path.to_string(),
        text: output,
        start_line,
        end_line,
        next_start_line,
        truncated,
        source_truncated,
    })
}

pub fn add(workspace: &Workspace, pathspec: &[String]) -> Result<GitResult, GitError> {
    validate_pathspec(workspace, pathspec, true)?;
    if pathspec.is_empty() {
        return Err(GitError::Invalid("add requires at least one path".into()));
    }
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(pathspec.iter().cloned());
    run_owned(workspace, &args)
}

pub fn commit(
    workspace: &Workspace,
    message: &str,
    pathspec: &[String],
) -> Result<GitResult, GitError> {
    validate_commit_message(message)?;
    validate_pathspec(workspace, pathspec, false)?;
    let mut args = vec!["commit".into(), "-m".into(), message.trim().to_string()];
    if !pathspec.is_empty() {
        args.push("--".into());
        args.extend(pathspec.iter().cloned());
    }
    run_owned(workspace, &args)
}

pub fn switch(workspace: &Workspace, branch: &str) -> Result<GitResult, GitError> {
    validate_ref_name(branch)?;
    run_owned(workspace, &["switch".into(), branch.into()])
}

pub fn create_branch(workspace: &Workspace, branch: &str) -> Result<GitResult, GitError> {
    validate_ref_name(branch)?;
    run_owned(workspace, &["switch".into(), "-c".into(), branch.into()])
}

pub fn restore(
    workspace: &Workspace,
    staged: bool,
    pathspec: &[String],
) -> Result<GitResult, GitError> {
    validate_pathspec(workspace, pathspec, false)?;
    if pathspec.is_empty() {
        return Err(GitError::Invalid(
            "restore requires at least one path".into(),
        ));
    }
    let mut args = vec!["restore".to_string()];
    if staged {
        args.push("--staged".into());
    }
    args.push("--".into());
    args.extend(pathspec.iter().cloned());
    run_owned(workspace, &args)
}

pub fn push(
    workspace: &Workspace,
    remote: Option<&str>,
    refspec: Option<&str>,
) -> Result<GitResult, GitError> {
    let remote = remote.unwrap_or("origin");
    validate_ref_name(remote)?;
    if let Some(refspec) = refspec {
        validate_ref_name(refspec)?;
    }
    let mut args = vec!["push".to_string(), remote.to_string()];
    if let Some(refspec) = refspec {
        args.push(refspec.to_string());
    }
    run_owned(workspace, &args)
}

pub fn mutation_argv(
    action: &str,
    staged: bool,
    pathspec: &[String],
    message: Option<&str>,
    branch: Option<&str>,
    remote: Option<&str>,
    refspec: Option<&str>,
) -> Result<Vec<String>, GitError> {
    match action {
        "add" => {
            if pathspec.is_empty() {
                return Err(GitError::Invalid("add requires at least one path".into()));
            }
            let mut args = vec!["git".into(), "add".into(), "--".into()];
            args.extend(pathspec.iter().cloned());
            Ok(args)
        }
        "commit" => {
            let message = message.unwrap_or_default();
            validate_commit_message(message)?;
            let mut args = vec![
                "git".into(),
                "commit".into(),
                "-m".into(),
                message.trim().to_string(),
            ];
            if !pathspec.is_empty() {
                args.push("--".into());
                args.extend(pathspec.iter().cloned());
            }
            Ok(args)
        }
        "switch" => {
            let branch = branch.unwrap_or_default();
            validate_ref_name(branch)?;
            Ok(vec!["git".into(), "switch".into(), branch.into()])
        }
        "create_branch" => {
            let branch = branch.unwrap_or_default();
            validate_ref_name(branch)?;
            Ok(vec![
                "git".into(),
                "switch".into(),
                "-c".into(),
                branch.into(),
            ])
        }
        "restore" => {
            if pathspec.is_empty() {
                return Err(GitError::Invalid(
                    "restore requires at least one path".into(),
                ));
            }
            let mut args = vec!["git".into(), "restore".into()];
            if staged {
                args.push("--staged".into());
            }
            args.push("--".into());
            args.extend(pathspec.iter().cloned());
            Ok(args)
        }
        "push" => {
            let remote = remote.unwrap_or("origin");
            validate_ref_name(remote)?;
            if let Some(refspec) = refspec {
                validate_ref_name(refspec)?;
            }
            let mut args = vec!["git".into(), "push".into(), remote.into()];
            if let Some(refspec) = refspec {
                args.push(refspec.into());
            }
            Ok(args)
        }
        _ => Err(GitError::Invalid("not a Git mutation action".into())),
    }
}

fn validate_pathspec(
    workspace: &Workspace,
    pathspec: &[String],
    _allow_new: bool,
) -> Result<(), GitError> {
    if pathspec.len() > 32 {
        return Err(GitError::Invalid(
            "pathspec may contain at most 32 paths".into(),
        ));
    }
    for path in pathspec {
        workspace
            .resolve_for_write(path)
            .map_err(GitError::Workspace)?;
    }
    Ok(())
}

fn split_diff_hunks(diff: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut prefix = String::new();
    let mut has_hunk = false;

    for line in diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if !current.is_empty() {
                chunks.push(current);
            }
            current = line.to_string();
            prefix = current.clone();
            has_hunk = false;
        } else if line.starts_with("@@ ") {
            if has_hunk {
                chunks.push(current);
            }
            current = std::mem::take(&mut prefix);
            current.push_str(line);
            has_hunk = true;
        } else {
            current.push_str(line);
            if !has_hunk {
                prefix.push_str(line);
            }
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn validate_commit_message(message: &str) -> Result<(), GitError> {
    let message = message.trim();
    if message.is_empty() || message.len() > 4096 {
        return Err(GitError::Invalid(
            "commit message must contain 1..=4096 bytes".into(),
        ));
    }
    Ok(())
}

fn validate_ref_name(value: &str) -> Result<(), GitError> {
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('-')
        || value.chars().any(|c| c.is_whitespace() || c == '\0')
    {
        return Err(GitError::Invalid("invalid Git ref or remote name".into()));
    }
    Ok(())
}

fn run_owned(workspace: &Workspace, args: &[String]) -> Result<GitResult, GitError> {
    run_owned_with_limit(workspace, args, MAX_OUTPUT)
}

fn run_owned_with_limit(
    workspace: &Workspace,
    args: &[String],
    stdout_limit: usize,
) -> Result<GitResult, GitError> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_with_stdout_limit(workspace, &refs, stdout_limit)
}

fn run(workspace: &Workspace, args: &[&str]) -> Result<GitResult, GitError> {
    run_with_stdout_limit(workspace, args, MAX_OUTPUT)
}

fn run_with_stdout_limit(
    workspace: &Workspace,
    args: &[&str],
    stdout_limit: usize,
) -> Result<GitResult, GitError> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(workspace.root())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    env::apply(&mut command);
    let child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GitError::Unavailable
        } else {
            GitError::Failed(error.to_string())
        }
    })?;
    let output = command_output::collect(child, stdout_limit, 64 * 1024)
        .map_err(|error| GitError::Failed(error.to_string()))?;
    if !output.status.success() {
        return Err(GitError::Failed(redact::text(
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }
    let bytes = output.stdout;
    let stdout_truncated = output.stdout_truncated;
    let truncated = stdout_truncated || output.stderr_truncated;
    let stderr = output.stderr;
    let mut stdout = redact::text(&String::from_utf8_lossy(&bytes));
    if !output.stdout_truncated && bytes.ends_with(b"\n") && !stdout.ends_with('\n') {
        stdout.push('\n');
    }
    Ok(GitResult {
        stdout,
        stderr: redact::text(&String::from_utf8_lossy(&stderr)),
        truncated,
        stdout_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn status_works_in_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success());
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let result = status(&workspace).unwrap();
        assert!(result.stdout.contains("a.txt"));
    }

    #[test]
    fn add_and_commit_are_structured() {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        add(&workspace, &["a.txt".into()]).unwrap();
        commit(&workspace, "test commit", &[]).unwrap();
        assert!(log(&workspace, 1).unwrap().stdout.contains("test commit"));
    }

    #[test]
    fn protected_diff_path_scan_returns_names_without_diff_contents() {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=old-secret\n").unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        add(&workspace, &[".env".into()]).unwrap();
        commit(&workspace, "add protected file", &[]).unwrap();
        fs::write(dir.path().join(".env"), "TOKEN=new-secret\n").unwrap();

        let paths = protected_diff_paths(&workspace, false, &[]).unwrap();
        assert_eq!(paths, vec![".env"]);
        let authorization = diff_authorization_argv(false, &[]);
        assert_eq!(authorization, vec!["git", "diff", "--no-color"]);
    }

    #[test]
    fn show_file_page_returns_parser_ready_line_continuations() {
        let dir = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        let expected = (1..=100)
            .map(|line| format!("line-{line:03}\n"))
            .collect::<String>();
        fs::write(dir.path().join("history.txt"), &expected).unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        add(&workspace, &["history.txt".into()]).unwrap();
        commit(&workspace, "file history", &[]).unwrap();

        let mut combined = String::new();
        let mut start_line = 1;
        loop {
            let page = show_file_page(&workspace, "HEAD", "history.txt", start_line, 64).unwrap();
            assert!(page.text.len() <= 64);
            combined.push_str(&page.text);
            if let Some(next) = page.next_start_line {
                assert!(next > start_line);
                start_line = next;
            } else {
                assert!(!page.truncated);
                break;
            }
        }
        assert_eq!(combined, expected);
    }

    #[test]
    fn mutation_argv_is_exact() {
        assert_eq!(
            mutation_argv("push", false, &[], None, None, Some("origin"), Some("main")).unwrap(),
            vec!["git", "push", "origin", "main"]
        );
        assert_eq!(
            mutation_argv(
                "commit",
                false,
                &["a.txt".into()],
                Some("message"),
                None,
                None,
                None
            )
            .unwrap(),
            vec!["git", "commit", "-m", "message", "--", "a.txt"]
        );
        assert_eq!(
            mutation_argv(
                "create_branch",
                false,
                &[],
                None,
                Some("feature/test"),
                None,
                None
            )
            .unwrap(),
            vec!["git", "switch", "-c", "feature/test"]
        );
    }

    #[test]
    fn diff_hunks_are_pageable_without_crossing_file_boundaries() {
        let diff = concat!(
            "diff --git a/a.txt b/a.txt\n",
            "index 1..2 100644\n",
            "--- a/a.txt\n",
            "+++ b/a.txt\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "+new\n",
            "@@ -4 +4 @@\n",
            "-old2\n",
            "+new2\n",
            "diff --git a/b.txt b/b.txt\n",
            "--- a/b.txt\n",
            "+++ b/b.txt\n",
            "@@ -1 +1 @@\n",
            "-x\n",
            "+y\n",
        );
        let chunks = split_diff_hunks(diff);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].starts_with("diff --git a/a.txt b/a.txt\n"));
        assert!(chunks[1].starts_with("@@ -4 +4 @@\n"));
        assert!(chunks[2].starts_with("diff --git a/b.txt b/b.txt\n"));
    }
}
