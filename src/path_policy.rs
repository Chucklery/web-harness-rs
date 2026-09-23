use std::path::Path;

pub fn is_protected_workspace_path(
    workspace: &crate::workspace::Workspace,
    path: impl AsRef<Path>,
) -> bool {
    let path = path.as_ref();
    if is_protected(path) {
        return true;
    }
    let Ok(resolved) = workspace.resolve(path) else {
        return false;
    };
    resolved
        .strip_prefix(workspace.root())
        .is_ok_and(is_protected)
}

pub const PROTECTED_GLOBS: &[&str] = &[
    "!**/.env",
    "!**/.env.*",
    "!**/*.pem",
    "!**/*.key",
    "!**/*.p12",
    "!**/*.pfx",
    "!**/*credentials*",
    "!**/*private*",
    "!**/*secret*",
    "!**/*password*",
    "!**/.netrc",
    "!**/.npmrc",
    "!**/.pypirc",
    "!**/.docker/config.json",
    "!**/.ssh/**",
];

pub const SAFE_EXAMPLE_GLOBS: &[&str] = &["**/.env.example", "**/.env.sample", "**/.env.template"];

pub const PROTECTED_DIRECTORY_GLOBS: &[&str] = &[
    "!**/.ssh/**",
    "!**/secret/**",
    "!**/secrets/**",
    "!**/credentials/**",
    "!**/private/**",
    "!**/password/**",
    "!**/passwords/**",
];

pub fn is_safe_example(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .is_some_and(|name| {
            matches!(
                name.as_str(),
                ".env.example" | ".env.sample" | ".env.template"
            )
        })
}

pub fn is_protected(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(".ssh"))
    }) {
        return true;
    }
    if path.components().any(|component| {
        component.as_os_str().to_str().is_some_and(|value| {
            [
                "secret",
                "secrets",
                "credentials",
                "private",
                "password",
                "passwords",
            ]
            .iter()
            .any(|name| value.eq_ignore_ascii_case(name))
        })
    }) {
        return true;
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    if is_safe_example(path) {
        return false;
    }
    if name == ".env"
        || name.starts_with(".env.")
        || has_token(&name, "credentials")
        || name.replace('-', "_").contains("private_key")
        || has_token(&name, "secret")
        || has_token(&name, "password")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.starts_with("id_dsa")
        || matches!(name.as_str(), ".netrc" | ".npmrc" | ".pypirc")
    {
        return true;
    }
    if name == "config.json"
        && path
            .parent()
            .and_then(|parent| parent.file_name())
            .is_some_and(|parent| parent.eq_ignore_ascii_case(".docker"))
    {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("pem" | "key" | "p12" | "pfx" | "jks")
    )
}

fn has_token(name: &str, token: &str) -> bool {
    name.split(|character: char| !character.is_ascii_alphanumeric())
        .any(|part| part == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_common_secret_files_without_blocking_examples() {
        assert!(is_protected(".env"));
        assert!(is_protected("config/private_key.pem"));
        assert!(is_protected(".ssh/id_ed25519"));
        assert!(is_protected("credentials.json"));
        assert!(is_protected("secrets/config.json"));
        assert!(is_protected("private/config.toml"));
        assert!(is_protected("Secrets/Config.json"));
        assert!(is_protected(".npmrc"));
        assert!(is_protected(".docker/config.json"));
        assert!(is_protected(".ssh/id_ecdsa"));
        assert!(!is_protected(".env.example"));
        assert!(!is_protected("src/secretary.rs"));
    }
}
