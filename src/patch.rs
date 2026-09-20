use crate::atomic_file;
use crate::workspace::{Workspace, WorkspaceError};
use std::fs;
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

    // Two-phase commit: every file is staged before any target is touched, so a
    // failure while preparing the third file cannot leave the first two already
    // rewritten. Without this a multi-file patch was only atomic per file.
    let mut staged = Vec::new();
    let mut changed = Vec::new();
    for (relative, target, content) in prepared {
        let result = match &content {
            Some(content) => atomic_file::stage(&target, content.as_bytes(), false),
            None => atomic_file::stage_removal(&target),
        };
        match result {
            Ok(entry) => {
                staged.push(entry);
                changed.push(relative);
            }
            Err(error) => {
                for entry in staged {
                    entry.discard();
                }
                return Err(PatchError::Io(error));
            }
        }
    }

    let mut applied = 0usize;
    for entry in staged {
        if let Err(error) = entry.commit() {
            // The files already committed stay committed; report the failure
            // rather than pretending the patch was atomic across all of them.
            return Err(PatchError::Io(error));
        }
        applied += 1;
    }
    debug_assert_eq!(applied, changed.len());

    Ok(changed)
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

    /// Creates a directory that rejects new files, or `None` when the process
    /// can still write into it (for example when running as root).
    ///
    /// The failure has to happen while staging, not while parsing or preparing:
    /// a bad hunk is rejected before any file is touched, so it cannot show
    /// whether a partially applied patch is rolled back.
    fn unwritable_dir(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        let dir = root.join(name);
        fs::create_dir(&dir).unwrap();
        let mut permissions = fs::metadata(&dir).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&dir, permissions).unwrap();
        if fs::write(dir.join(".probe"), b"x").is_ok() {
            let _ = fs::remove_file(dir.join(".probe"));
            restore_dir(&dir);
            return None;
        }
        Some(dir)
    }

    fn restore_dir(dir: &std::path::Path) {
        let mut permissions = fs::metadata(dir).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o755);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        fs::set_permissions(dir, permissions).unwrap();
    }

    #[test]
    fn a_late_failure_leaves_earlier_files_untouched() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "old\n").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let Some(blocked) = unwritable_dir(dir.path(), "blocked") else {
            return;
        };
        // `a.txt` stages cleanly; writing into `blocked/` cannot.
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** Add File: blocked/new.txt\n+created\n*** End Patch";
        assert!(apply(&ws, patch).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "old\n",
            "a.txt must not be rewritten when a sibling operation cannot be staged"
        );
        restore_dir(&blocked);
    }

    #[test]
    fn a_late_failure_restores_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "still here\n").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let Some(blocked) = unwritable_dir(dir.path(), "blocked") else {
            return;
        };
        let patch = "*** Begin Patch\n*** Delete File: keep.txt\n*** Add File: blocked/new.txt\n+created\n*** End Patch";
        assert!(apply(&ws, patch).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
            "still here\n",
            "a delete must be rolled back when a sibling operation cannot be staged"
        );
        restore_dir(&blocked);
    }

    #[test]
    fn staging_leaves_no_temporary_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "old\n").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let Some(blocked) = unwritable_dir(dir.path(), "blocked") else {
            return;
        };
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** Add File: blocked/new.txt\n+created\n*** End Patch";
        assert!(apply(&ws, patch).is_err());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".web-harness-"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
        restore_dir(&blocked);
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
