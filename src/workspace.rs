use serde::Serialize;
use sha2::{Digest, Sha256};
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
    #[error("path is excluded by the workspace boundary: {0}")]
    Denied(PathBuf),
    #[error("invalid boundary entry: {0}")]
    InvalidBoundary(String),
    #[error("failed to access path: {0}")]
    Io(#[from] std::io::Error),
}

/// Paths that are inside the workspace but never worth handing to a remote
/// agent: version-control internals and build output. They are excluded by
/// default so that connecting in a project directory does not implicitly
/// expose the whole checkout, including uncommitted history.
pub const DEFAULT_DENY_PATHS: &[&str] = &[".git", "target", "node_modules"];

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
    denied: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceInfo {
    pub root: String,
    pub denied: Vec<String>,
}

impl Workspace {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        Self::with_denied(root, &[])
    }

    /// Builds a workspace whose boundary additionally excludes `denied`.
    pub fn with_denied(root: impl AsRef<Path>, denied: &[String]) -> Result<Self, WorkspaceError> {
        let root = root.as_ref();
        if !root.exists() {
            return Err(WorkspaceError::NotFound(root.to_path_buf()));
        }
        if !root.is_dir() {
            return Err(WorkspaceError::NotDirectory(root.to_path_buf()));
        }
        let denied = denied
            .iter()
            .map(|entry| validate_boundary_entry(entry))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            root: fs::canonicalize(root)?,
            denied,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn denied(&self) -> Vec<String> {
        self.denied
            .iter()
            .map(|entry| entry.display().to_string())
            .collect()
    }

    pub fn info(&self) -> WorkspaceInfo {
        WorkspaceInfo {
            root: self.root.display().to_string(),
            denied: self.denied(),
        }
    }

    /// True when `canonical` sits inside an excluded subtree.
    fn is_denied(&self, canonical: &Path) -> bool {
        let Ok(relative) = canonical.strip_prefix(&self.root) else {
            return false;
        };
        self.denied
            .iter()
            .any(|denied| relative == denied || relative.starts_with(denied))
    }

    pub fn resolve(&self, relative: impl AsRef<Path>) -> Result<PathBuf, WorkspaceError> {
        let canonical = fs::canonicalize(self.root.join(relative.as_ref()))?;
        if !canonical.starts_with(&self.root) {
            return Err(WorkspaceError::OutsideWorkspace(canonical));
        }
        if self.is_denied(&canonical) {
            return Err(WorkspaceError::Denied(canonical));
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
        let resolved = canonical_parent.join(
            candidate
                .file_name()
                .ok_or_else(|| WorkspaceError::OutsideWorkspace(candidate.clone()))?,
        );
        if self.is_denied(&resolved) {
            return Err(WorkspaceError::Denied(resolved));
        }
        Ok(resolved)
    }

    pub fn ensure_parent_dirs(
        &self,
        relative: impl AsRef<Path>,
    ) -> Result<Vec<PathBuf>, WorkspaceError> {
        let relative = relative.as_ref();
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(WorkspaceError::OutsideWorkspace(relative.to_path_buf()));
        }
        let mut current = self.root.clone();
        let mut created = Vec::new();
        let components = relative.components().collect::<Vec<_>>();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    cleanup_created_dirs(&created);
                    return Err(WorkspaceError::Denied(current));
                }
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    cleanup_created_dirs(&created);
                    return Err(WorkspaceError::Io(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "parent path is not a directory",
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if self.is_denied(&current) {
                        cleanup_created_dirs(&created);
                        return Err(WorkspaceError::Denied(current));
                    }
                    if let Err(error) = fs::create_dir(&current) {
                        cleanup_created_dirs(&created);
                        return Err(WorkspaceError::Io(error));
                    }
                    created.push(current.clone());
                }
                Err(error) => {
                    cleanup_created_dirs(&created);
                    return Err(WorkspaceError::Io(error));
                }
            }
        }
        Ok(created)
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

    pub fn read_revision(&self, relative: impl AsRef<Path>) -> Result<String, WorkspaceError> {
        let path = self.resolve(relative)?;
        let mut file = fs::File::open(path)?;
        let length = file.metadata()?.len();
        let mut digest = Sha256::new();
        digest.update(length.to_le_bytes());
        std::io::copy(&mut file, &mut digest)?;
        Ok(format!("sha256:{:x}", digest.finalize()))
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

fn cleanup_created_dirs(created: &[PathBuf]) {
    for directory in created.iter().rev() {
        let _ = fs::remove_dir(directory);
    }
}

/// Accepts only simple relative paths so that a boundary entry can never widen
/// the workspace or name something outside it.
pub fn validate_boundary_entry(entry: &str) -> Result<PathBuf, WorkspaceError> {
    let entry = entry.trim();
    let path = Path::new(entry);
    if entry.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(WorkspaceError::InvalidBoundary(entry.to_string()));
    }
    Ok(path.to_path_buf())
}

/// Parses a comma-separated boundary list; an empty string means "no exclusions".
pub fn parse_deny_paths(input: &str) -> Result<Vec<String>, WorkspaceError> {
    input
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            validate_boundary_entry(entry)?;
            Ok(entry.to_string())
        })
        .collect()
}

/// Boundary list in effect for a server: an explicit list wins, otherwise the
/// environment override, otherwise the conservative default.
pub fn effective_deny_paths(explicit: Option<&str>) -> Result<Vec<String>, WorkspaceError> {
    match explicit {
        Some(value) => parse_deny_paths(value),
        None => match std::env::var("WEB_HARNESS_DENY_PATHS") {
            Ok(value) => parse_deny_paths(&value),
            Err(_) => Ok(DEFAULT_DENY_PATHS.iter().map(|s| s.to_string()).collect()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_excludes_default_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/config"), "x").unwrap();
        fs::write(dir.path().join("src.txt"), "ok").unwrap();
        let denied: Vec<String> = DEFAULT_DENY_PATHS.iter().map(|s| s.to_string()).collect();
        let workspace = Workspace::with_denied(dir.path(), &denied).unwrap();

        assert!(workspace.resolve(".git/config").is_err());
        assert!(workspace.resolve("src.txt").is_ok());
        assert_eq!(workspace.denied(), vec![".git", "target", "node_modules"]);
    }

    #[test]
    fn boundary_blocks_writes_into_excluded_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        let workspace = Workspace::with_denied(dir.path(), &["target".to_string()]).unwrap();

        assert!(workspace.resolve_for_write("target/out.bin").is_err());
        assert!(workspace.resolve_for_write("src/out.txt").is_ok());
    }

    #[test]
    fn boundary_entries_cannot_widen_the_workspace() {
        assert!(validate_boundary_entry("../escape").is_err());
        assert!(validate_boundary_entry("/etc").is_err());
        assert!(validate_boundary_entry("").is_err());
        assert!(validate_boundary_entry("nested/dir").is_ok());
        assert!(parse_deny_paths("").unwrap().is_empty());
        assert_eq!(
            parse_deny_paths(".git, target").unwrap(),
            vec![".git".to_string(), "target".to_string()]
        );
    }

    #[test]
    fn workspace_info_reports_the_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::with_denied(dir.path(), &["target".to_string()]).unwrap();
        assert_eq!(workspace.info().denied, vec!["target".to_string()]);
    }

    #[test]
    fn resolves_paths_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "ok").unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        assert!(ws.resolve("a.txt").unwrap().starts_with(ws.root()));
    }

    #[test]
    fn read_rejects_parent_escape() {
        let outer = tempfile::tempdir().unwrap();
        let workspace_dir = outer.path().join("workspace");
        fs::create_dir(&workspace_dir).unwrap();
        fs::write(outer.path().join("outside.txt"), "secret").unwrap();
        let ws = Workspace::new(&workspace_dir).unwrap();
        assert!(matches!(
            ws.read_text_bounded("../outside.txt", 1024).unwrap_err(),
            WorkspaceError::OutsideWorkspace(_)
        ));
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

    #[cfg(unix)]
    #[test]
    fn write_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        let ws = Workspace::new(root.path()).unwrap();
        assert!(matches!(
            ws.resolve_for_write("escape/new.txt").unwrap_err(),
            WorkspaceError::OutsideWorkspace(_)
        ));
        assert!(!outside.path().join("new.txt").exists());
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
