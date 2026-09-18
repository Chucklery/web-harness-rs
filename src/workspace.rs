use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace does not exist: {0}")]
    NotFound(PathBuf),
    #[error("workspace is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("path is outside the workspace: {0}")]
    OutsideWorkspace(PathBuf),
    #[error("failed to access path: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceInfo {
    pub root: String,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = root.as_ref();
        if !root.exists() {
            return Err(WorkspaceError::NotFound(root.to_path_buf()));
        }
        if !root.is_dir() {
            return Err(WorkspaceError::NotDirectory(root.to_path_buf()));
        }
        Ok(Self {
            root: fs::canonicalize(root)?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo {
            root: self.root.display().to_string(),
        }
    }

    pub fn resolve(&self, relative: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let canonical = fs::canonicalize(self.root.join(relative.as_ref()))?;
        if !canonical.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideWorkspace(canonical));
        }
        Ok(canonical)
    }

    pub fn resolve_for_write(&self, relative: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let relative = relative.as_ref();
        if relative.is_absolute() {
            return Err(WorkspaceError::OutsideWorkspace(relative.to_path_buf()));
        }
        let candidate = self.root.join(relative);
        if candidate.exists() {
            return self.resolve(relative);
        }
        let parent = candidate.parent().unwrap_or(&self.root);
        let canonical_parent = fs::canonicalize(parent)?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideWorkspace(candidate));
        }
        Ok(canonical_parent.join(
            candidate
                .file_name()
                .ok_or_else(|| WorkspaceError::OutsideWorkspace(candidate.clone()))?,
        ))
    }

    pub fn read_text_bounded(
        &self,
        relative: impl AsRef<Path>,
        max_bytes: usize,
    ) -> Result<String, WorkspaceError> {
        let path = self.resolve(relative)?;
        if fs::metadata(&path)?.len() > max_bytes as u64 {
            return Err(WorkspaceError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("file exceeds read limit of {max_bytes} bytes"),
            )));
        }
        Ok(fs::read_to_string(path)?)
    }

    pub fn discover_agents(
        &self,
        relative: impl AsRef<Path>,
    ) -> Result<Vec<PathBuf>, WorkspaceError> {
        let target = self.resolve(relative)?;
        let mut current = if target.is_dir() {
            target
        } else {
            target.parent().unwrap_or(&self.root).to_path_buf()
        };
        let mut found = Vec::new();
        loop {
            let candidate = current.join("AGENTS.md");
            if candidate.is_file() {
                found.push(candidate);
            }
            if current == self.root {
                break;
            }
            let Some(parent) = current.parent() else {
                break;
            };
            if !parent.starts_with(&self.root) {
                break;
            }
            current = parent.to_path_buf();
        }
        found.reverse();
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_paths_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "ok").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        assert!(ws.resolve("a.txt").unwrap().starts_with(ws.root()));
    }

    #[test]
    fn rejects_symlink_escape() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("secret.txt"), "secret").unwrap();
            symlink(outside.path(), root.path().join("escape")).unwrap();
            let ws = Workspace::new(root.path()).unwrap();
            assert!(matches!(
                ws.resolve("escape/secret.txt").unwrap_err(),
                WorkspaceError::OutsideWorkspace(_)
            ));
        }
    }

    #[test]
    fn enforces_read_limit() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("big.txt"), "1234567890").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        assert!(ws.read_text_bounded("big.txt", 4).is_err());
    }

    #[test]
    fn discovers_scoped_agents_from_root_to_leaf() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/nested")).unwrap();
        fs::write(dir.path().join("AGENTS.md"), "root").unwrap();
        fs::write(dir.path().join("src/AGENTS.md"), "src").unwrap();
        fs::write(dir.path().join("src/nested/file.rs"), "").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let agents = ws.discover_agents("src/nested/file.rs").unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents[0].ends_with("AGENTS.md"));
        assert!(agents[1].ends_with("src/AGENTS.md"));
    }

    #[test]
    fn write_resolution_rejects_parent_escape() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        assert!(ws.resolve_for_write("../outside.txt").is_err());
    }
}
