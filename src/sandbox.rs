use crate::workspace::Workspace;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    MacOsSeatbelt,
    Unavailable,
}

impl SandboxBackend {
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            if Path::new("/usr/bin/sandbox-exec").is_file() {
                return Self::MacOsSeatbelt;
            }
        }
        Self::Unavailable
    }

    pub fn enforced(self) -> bool {
        matches!(self, Self::MacOsSeatbelt)
    }
}

pub fn status() -> &'static str {
    match SandboxBackend::detect() {
        SandboxBackend::MacOsSeatbelt => "macOS Seatbelt available; exec is sandboxed by default",
        SandboxBackend::Unavailable => {
            "no native sandbox backend available; exec requires explicit approval"
        }
    }
}

pub fn wrap_argv(workspace: &Workspace, argv: &[String]) -> Vec<String> {
    match SandboxBackend::detect() {
        SandboxBackend::MacOsSeatbelt => {
            let profile = macos_profile(workspace.root());
            let mut wrapped = Vec::with_capacity(argv.len() + 3);
            wrapped.push("/usr/bin/sandbox-exec".to_string());
            wrapped.push("-p".to_string());
            wrapped.push(profile);
            wrapped.extend(argv.iter().cloned());
            wrapped
        }
        SandboxBackend::Unavailable => argv.to_vec(),
    }
}

#[cfg(target_os = "macos")]
fn macos_profile(workspace: &Path) -> String {
    fn quoted(path: &Path) -> String {
        let escaped = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!("\"{escaped}\"")
    }

    let workspace = quoted(workspace);
    let temp_dir = std::env::temp_dir();
    let tmp = quoted(&temp_dir);
    let canonical_tmp = temp_dir
        .canonicalize()
        .ok()
        .filter(|path| path != &temp_dir)
        .map(|path| format!(" (subpath {})", quoted(&path)))
        .unwrap_or_default();
    let private_tmp = temp_dir
        .to_string_lossy()
        .strip_prefix("/var/")
        .map(|suffix| {
            let path = std::path::PathBuf::from(format!("/private/var/{suffix}"));
            format!(" (subpath {})", quoted(&path))
        })
        .unwrap_or_default();
    format!(
        "(version 1) \
         (deny default) \
         (allow process*) \
         (allow signal) \
         (allow sysctl-read) \
         (allow mach-lookup) \
         (allow file-read*) \
         (allow file-write* (subpath {workspace}) (subpath {tmp}){canonical_tmp}{private_tmp} (subpath \"/private/tmp\") (subpath \"/tmp\"))"
    )
}

#[cfg(not(target_os = "macos"))]
fn macos_profile(_workspace: &Path) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_detection_is_explicit() {
        assert!(matches!(
            SandboxBackend::detect(),
            SandboxBackend::MacOsSeatbelt | SandboxBackend::Unavailable
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn generated_profile_mentions_workspace_and_denies_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let wrapped = wrap_argv(&ws, &["/bin/echo".into(), "ok".into()]);
        assert_eq!(wrapped[0], "/usr/bin/sandbox-exec");
        assert!(wrapped[2].contains("(deny default)"));
        assert!(wrapped[2].contains(&dir.path().display().to_string()));
    }
}
