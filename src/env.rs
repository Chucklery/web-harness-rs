//! Minimal environment inheritance for child processes.
//!
//! Every child web-harness spawns starts from an empty environment plus this
//! allowlist. Inheriting the parent's environment wholesale would leak
//! credentials into `exec`, `git`, and `search` alike, and `redact` only scrubs
//! output — so the environment has to be bounded at the source rather than
//! cleaned up afterwards.

use std::ffi::OsString;
use std::process::Command;

use crate::redact;

/// Variables a child process may inherit.
///
/// `LC_*` is handled separately in [`safe_environment`] because it is a family
/// rather than a fixed set of names.
const SAFE_KEYS: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "TERM",
    "SHELL",
    "USER",
    "LOGNAME",
    "SSH_AUTH_SOCK",
];

/// Returns the allowlisted subset of the current environment.
///
/// `LC_*` variables are inherited because they carry locale rather than
/// credentials, but a name that looks secret is dropped even if it matches the
/// prefix.
pub fn safe_environment() -> Vec<(OsString, OsString)> {
    let mut result = Vec::new();
    for key in SAFE_KEYS {
        if let Some(value) = std::env::var_os(key) {
            result.push((OsString::from(key), value));
        }
    }
    for (key, value) in std::env::vars_os() {
        let text = key.to_string_lossy();
        if text.starts_with("LC_") && !redact::sensitive_env_key(&text) {
            result.push((key, value));
        }
    }
    result
}

/// Clears `command`'s environment and applies [`safe_environment`].
pub fn apply(command: &mut Command) {
    command.env_clear();
    command.envs(safe_environment());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherits_only_allowlisted_or_locale_keys() {
        for (key, _) in safe_environment() {
            let text = key.to_string_lossy();
            let allowed = SAFE_KEYS.contains(&text.as_ref())
                || (text.starts_with("LC_") && !redact::sensitive_env_key(&text));
            assert!(allowed, "unexpected key inherited: {text}");
        }
    }

    #[test]
    fn never_inherits_a_secret_shaped_key() {
        for (key, _) in safe_environment() {
            assert!(!redact::sensitive_env_key(&key.to_string_lossy()));
        }
    }
}
