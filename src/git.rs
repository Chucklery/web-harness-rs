use crate::workspace::Workspace;
use serde::Serialize;
use std::process::Command;
use thiserror::Error;

const MAX_OUTPUT: usize = 512 * 1024;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git is not available")]
    Unavailable,
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
}

pub fn status(workspace: &Workspace) -> Result<GitResult, GitError> {
    run(workspace, &["status", "--short", "--branch"])
}

pub fn diff(
    workspace: &Workspace,
    staged: bool,
    pathspec: &[String],
) -> Result<GitResult, GitError> {
    if pathspec.len() > 32 {
        return Err(GitError::Invalid(
            "pathspec may contain at most 32 paths".into(),
        ));
    }
    let mut args = vec!["diff".to_string()];
    if staged {
        args.push("--cached".into());
    }
    if !pathspec.is_empty() {
        args.push("--".into());
        for path in pathspec {
            workspace
                .resolve(path)
                .map_err(|error| GitError::Invalid(error.to_string()))?;
            args.push(path.clone());
        }
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(workspace, &refs)
}

pub fn log(workspace: &Workspace, limit: u64) -> Result<GitResult, GitError> {
    let limit = limit.clamp(1, 100).to_string();
    run(
        workspace,
        &["log", "--oneline", "--decorate", "--no-color", "-n", &limit],
    )
}

pub fn show(workspace: &Workspace, revision: &str) -> Result<GitResult, GitError> {
    if revision.is_empty() || revision.len() > 256 || revision.starts_with('-') {
        return Err(GitError::Invalid("invalid revision".into()));
    }
    run(
        workspace,
        &["show", "--stat", "--oneline", "--no-color", "--", revision],
    )
    .or_else(|_| {
        run(
            workspace,
            &["show", "--stat", "--oneline", "--no-color", revision],
        )
    })
}

fn run(workspace: &Workspace, args: &[&str]) -> Result<GitResult, GitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace.root())
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                GitError::Unavailable
            } else {
                GitError::Failed(error.to_string())
            }
        })?;
    if !output.status.success() {
        return Err(GitError::Failed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let mut bytes = output.stdout;
    let truncated = bytes.len() > MAX_OUTPUT;
    if truncated {
        bytes.truncate(MAX_OUTPUT);
    }
    let mut stderr = output.stderr;
    if stderr.len() > 64 * 1024 {
        stderr.truncate(64 * 1024);
    }
    Ok(GitResult {
        stdout: String::from_utf8_lossy(&bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        truncated,
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
}
