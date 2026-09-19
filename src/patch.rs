use crate::atomic_file;
use crate::workspace::{Workspace, WorkspaceError};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatchError {
    #[error("invalid patch: {0}")]
    Invalid(String),
    #[error("patch conflict in {0}")]
    Conflict(String),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
enum Operation {
    Add { path: String, content: String },
    Update { path: String, hunks: Vec<Hunk> },
    Delete { path: String },
}

#[derive(Debug)]
struct Hunk {
    old: String,
    new: String,
}

pub fn apply(workspace: &Workspace, input: &str) -> Result<Vec<String>, PatchError> {
    let operations = parse(input)?;
    let mut prepared = Vec::new();
    for operation in operations {
        match operation {
            Operation::Add { path, content } => {
                let target = workspace.resolve_for_write(&path)?;
                if target.exists() {
                    return Err(PatchError::Conflict(path));
                }
                prepared.push((path, target, Some(content)));
            }
            Operation::Update { path, hunks } => {
                let target = workspace.resolve(&path)?;
                let mut content = fs::read_to_string(&target)?;
                for hunk in hunks {
                    let Some(index) = content.find(&hunk.old) else {
                        return Err(PatchError::Conflict(path.clone()));
                    };
                    if !hunk.old.is_empty() && content[index + hunk.old.len()..].contains(&hunk.old)
                    {
                        return Err(PatchError::Invalid(format!(
                            "ambiguous hunk in {path}; provide more context"
                        )));
                    }
                    content.replace_range(index..index + hunk.old.len(), &hunk.new);
                }
                prepared.push((path, target, Some(content)));
            }
            Operation::Delete { path } => {
                let target = workspace.resolve(&path)?;
                prepared.push((path, target, None));
            }
        }
    }

    let mut changed = Vec::new();
    for (relative, target, content) in prepared {
        match content {
            Some(content) => atomic_write(&target, content.as_bytes())?,
            None => fs::remove_file(&target)?,
        }
        changed.push(relative);
    }
    Ok(changed)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    atomic_file::write(path, bytes, false)
}

fn parse(input: &str) -> Result<Vec<Operation>, PatchError> {
    let lines: Vec<&str> = input.lines().collect();
    if lines.first().copied() != Some("*** Begin Patch")
        || lines.last().copied() != Some("*** End Patch")
    {
        return Err(PatchError::Invalid(
            "patch must start with *** Begin Patch and end with *** End Patch".into(),
        ));
    }
    let mut operations = Vec::new();
    let mut index = 1;
    while index + 1 < lines.len() {
        let line = lines[index];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            index += 1;
            let mut content = String::new();
            while index + 1 < lines.len() && !lines[index].starts_with("*** ") {
                let value = lines[index].strip_prefix('+').ok_or_else(|| {
                    PatchError::Invalid("add-file lines must start with +".into())
                })?;
                content.push_str(value);
                content.push('\n');
                index += 1;
            }
            operations.push(Operation::Add {
                path: path.to_string(),
                content,
            });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            operations.push(Operation::Delete {
                path: path.to_string(),
            });
            index += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            index += 1;
            let mut hunks = Vec::new();
            while index + 1 < lines.len() && !lines[index].starts_with("*** ") {
                if lines[index].starts_with("@@") {
                    index += 1;
                }
                let mut old = String::new();
                let mut new = String::new();
                while index + 1 < lines.len()
                    && !lines[index].starts_with("@@")
                    && !lines[index].starts_with("*** ")
                {
                    let current = lines[index];
                    if current.is_empty() {
                        return Err(PatchError::Invalid("empty raw hunk line".into()));
                    }
                    let (prefix, value) = current.split_at(1);
                    match prefix {
                        " " => {
                            old.push_str(value);
                            old.push('\n');
                            new.push_str(value);
                            new.push('\n');
                        }
                        "-" => {
                            old.push_str(value);
                            old.push('\n');
                        }
                        "+" => {
                            new.push_str(value);
                            new.push('\n');
                        }
                        _ => {
                            return Err(PatchError::Invalid(format!(
                                "invalid hunk line: {current}"
                            )));
                        }
                    }
                    index += 1;
                }
                if old.is_empty() && new.is_empty() {
                    return Err(PatchError::Invalid("empty update hunk".into()));
                }
                hunks.push(Hunk { old, new });
            }
            operations.push(Operation::Update {
                path: path.to_string(),
                hunks,
            });
        } else {
            return Err(PatchError::Invalid(format!("unexpected line: {line}")));
        }
    }
    Ok(operations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_add_update_delete() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "old\nkeep\n").unwrap();
        fs::write(dir.path().join("delete.txt"), "gone").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n keep\n*** Add File: b.txt\n+created\n*** Delete File: delete.txt\n*** End Patch";
        let changed = apply(&ws, patch).unwrap();
        assert_eq!(changed.len(), 3);
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "new\nkeep\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("b.txt")).unwrap(),
            "created\n"
        );
        assert!(!dir.path().join("delete.txt").exists());
    }

    #[test]
    fn rejects_conflicting_update() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "actual\n").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-missing\n+new\n*** End Patch";
        assert!(matches!(apply(&ws, patch), Err(PatchError::Conflict(_))));
    }
}
