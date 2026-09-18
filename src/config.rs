use crate::redact;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub schema_version: u32,
    pub workspace: String,
    pub tunnel_command: Option<Vec<String>>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("HOME is not available")]
    MissingHome,
    #[error("invalid tunnel command: {0}")]
    InvalidTunnel(String),
}

impl UserConfig {
    pub fn from_inputs(
        workspace: &Path,
        tunnel_command_json: Option<&str>,
    ) -> Result<Self, ConfigError> {
        Ok(Self {
            schema_version: 1,
            workspace: workspace.display().to_string(),
            tunnel_command: tunnel_command_json.map(parse_tunnel_command).transpose()?,
        })
    }

    pub fn from_wrapper(workspace: &Path, wrapper: &Path) -> Result<Self, ConfigError> {
        if !wrapper.is_absolute() || !wrapper.is_file() {
            return Err(ConfigError::InvalidTunnel(
                "tunnel wrapper must be an absolute path to an existing file".into(),
            ));
        }
        Ok(Self {
            schema_version: 1,
            workspace: workspace.display().to_string(),
            tunnel_command: Some(vec![wrapper.display().to_string()]),
        })
    }
}

pub fn load_optional() -> Result<Option<UserConfig>, ConfigError> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

pub fn save(config: &UserConfig) -> Result<PathBuf, ConfigError> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(config)?)?;
    fs::rename(temp, &path)?;
    Ok(path)
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(root).join("web-harness/config.json"));
    }
    let home = std::env::var_os("HOME").ok_or(ConfigError::MissingHome)?;
    Ok(PathBuf::from(home).join(".config/web-harness/config.json"))
}

pub fn state_dir() -> Result<PathBuf, ConfigError> {
    if let Some(root) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(root).join("web-harness"));
    }
    let home = std::env::var_os("HOME").ok_or(ConfigError::MissingHome)?;
    Ok(PathBuf::from(home).join(".local/state/web-harness"))
}

pub fn parse_tunnel_command(input: &str) -> Result<Vec<String>, ConfigError> {
    let argv: Vec<String> = serde_json::from_str(input)?;
    if argv.is_empty() || argv.len() > 64 || argv.iter().any(|arg| arg.len() > 16 * 1024) {
        return Err(ConfigError::InvalidTunnel(
            "expected a non-empty bounded JSON argv array".into(),
        ));
    }
    for arg in &argv {
        let lower = arg.to_ascii_lowercase();
        let key = lower
            .split_once('=')
            .map(|(key, _)| key)
            .unwrap_or_default();
        if lower.contains("bearer ")
            || lower.contains("password=")
            || lower.contains("token=")
            || lower.contains("api_key=")
            || lower.contains("apikey=")
            || lower.contains("cookie=")
            || redact::sensitive_env_key(key)
        {
            return Err(ConfigError::InvalidTunnel(
                "command appears to contain credentials; use official login state or environment instead of storing secrets in config".into(),
            ));
        }
    }
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_secret_bearing_tunnel_argv() {
        let error = parse_tunnel_command(r#"["wrapper","--token=secret"]"#).unwrap_err();
        assert!(error.to_string().contains("credentials"));
    }

    #[test]
    fn accepts_wrapper_without_credentials() {
        assert_eq!(
            parse_tunnel_command(r#"["/usr/local/bin/tunnel-wrapper","--profile","default"]"#)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn accepts_absolute_existing_wrapper_path() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("wrapper");
        std::fs::write(&wrapper, "").unwrap();
        let config = UserConfig::from_wrapper(dir.path(), &wrapper).unwrap();
        assert_eq!(
            config.tunnel_command.unwrap(),
            vec![wrapper.display().to_string()]
        );
    }
}
