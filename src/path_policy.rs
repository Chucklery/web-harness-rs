use std::path::Path;

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
    "!**/.ssh/**",
];

pub fn is_protected(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if path
        .components()
        .any(|component| component.as_os_str() == ".ssh")
    {
        return true;
    }
    if path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("secret" | "secrets" | "credentials" | "private" | "password" | "passwords")
        )
    }) {
        return true;
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        ".env.example" | ".env.sample" | ".env.template"
    ) {
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
        assert!(!is_protected(".env.example"));
        assert!(!is_protected("src/secretary.rs"));
    }
}
