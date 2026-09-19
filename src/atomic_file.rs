use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

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
