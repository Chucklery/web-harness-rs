use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// A fully written temporary file that has not yet replaced its target.
///
/// Callers that must change several files together stage every write first and
/// only then commit, so a failure while preparing one file cannot leave the
/// others half-applied.
pub struct Staged {
    temp_path: std::path::PathBuf,
    target: std::path::PathBuf,
    kind: StagedKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StagedKind {
    /// A new body is waiting to replace the target's current contents.
    Replace,
    /// The target was moved aside and is waiting to be dropped.
    Removal,
}

impl Staged {
    /// Applies the staged change to the target path.
    pub fn commit(self) -> io::Result<()> {
        match self.kind {
            StagedKind::Replace => {
                let parent = self.target.parent().unwrap_or_else(|| Path::new("."));
                #[cfg(windows)]
                if self.target.exists() {
                    fs::remove_file(&self.target)?;
                }
                fs::rename(&self.temp_path, &self.target)?;
                if let Ok(parent_dir) = File::open(parent) {
                    let _ = parent_dir.sync_all();
                }
                Ok(())
            }
            StagedKind::Removal => fs::remove_file(&self.temp_path),
        }
    }

    /// Undoes the staged change, leaving the target as it was.
    pub fn discard(self) {
        match self.kind {
            StagedKind::Replace => {
                let _ = fs::remove_file(&self.temp_path);
            }
            // The target was moved aside, so putting it back is the rollback.
            StagedKind::Removal => {
                let _ = fs::rename(&self.temp_path, &self.target);
            }
        }
    }
}

/// Moves `path` aside so it can be deleted as part of a multi-file commit.
///
/// The target is absent until the returned [`Staged`] is committed, which is
/// what makes it possible to restore the file if a sibling change fails.
pub fn stage_removal(path: &Path) -> io::Result<Staged> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("web-harness"))
        .to_string_lossy();

    for _ in 0..16 {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix = u64::from_ne_bytes(nonce);
        let temp_path = parent.join(format!(".{name}.web-harness-{suffix:016x}.rm"));

        match fs::symlink_metadata(&temp_path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        fs::rename(path, &temp_path)?;
        return Ok(Staged {
            temp_path,
            target: path.to_path_buf(),
            kind: StagedKind::Removal,
        });
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate removal staging path",
    ))
}

/// Writes `bytes` to a temporary file next to `path` without replacing it.
///
/// The returned [`Staged`] must be either committed or discarded.
pub fn stage(path: &Path, bytes: &[u8], private: bool) -> io::Result<Staged> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("web-harness"))
        .to_string_lossy();

    for _ in 0..16 {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix = u64::from_ne_bytes(nonce);
        let temp_path = parent.join(format!(".{name}.web-harness-{suffix:016x}.tmp"));

        let mut temp = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| {
            temp.write_all(bytes)?;
            temp.sync_all()?;

            #[cfg(unix)]
            if private {
                use std::os::unix::fs::PermissionsExt;
                temp.set_permissions(fs::Permissions::from_mode(0o600))?;
            }

            drop(temp);
            Ok(())
        })();

        return match result {
            Ok(()) => Ok(Staged {
                temp_path,
                target: path.to_path_buf(),
                kind: StagedKind::Replace,
            }),
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                Err(error)
            }
        };
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate atomic-write temporary file",
    ))
}

pub fn write(path: &Path, bytes: &[u8], private: bool) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new("web-harness"))
        .to_string_lossy();

    for _ in 0..16 {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let suffix = u64::from_ne_bytes(nonce);
        let temp_path = parent.join(format!(".{name}.web-harness-{suffix:016x}.tmp"));

        let mut temp = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| {
            temp.write_all(bytes)?;
            temp.sync_all()?;

            #[cfg(unix)]
            if private {
                use std::os::unix::fs::PermissionsExt;
                temp.set_permissions(fs::Permissions::from_mode(0o600))?;
            }

            drop(temp);

            #[cfg(windows)]
            if path.exists() {
                fs::remove_file(path)?;
            }

            fs::rename(&temp_path, path)?;
            if let Ok(parent_dir) = File::open(parent) {
                let _ = parent_dir.sync_all();
            }
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        return result;
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate atomic-write temporary file",
    ))
}
