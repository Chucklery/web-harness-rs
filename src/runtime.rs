use crate::config::{self, UserConfig};
use crate::onboarding;
use crate::sandbox;
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeState {
    schema_version: u32,
    tunnel_pid: u32,
    workspace: String,
    started_unix_s: u64,
}

#[derive(Debug, Serialize)]
pub struct UserStatus {
    pub configured: bool,
    pub connected: bool,
    pub stale_state_recovered: bool,
    pub workspace: Option<String>,
    pub tunnel_pid: Option<u32>,
    pub sandbox: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("web-harness is not configured; run setup first")]
    NotConfigured,
    #[error("tunnel command is not configured; run setup with --tunnel-command-json")]
    TunnelNotConfigured,
    #[error("configured workspace is invalid: {0}")]
    InvalidWorkspace(String),
}

pub fn connect(
    workspace_override: Option<&Path>,
    tunnel_override: Option<&str>,
) -> Result<UserStatus, RuntimeError> {
    let mut user_config = config::load_optional()?.ok_or(RuntimeError::NotConfigured)?;
    let selected_workspace = match workspace_override {
        Some(workspace) => workspace.to_path_buf(),
        None => std::env::current_dir()?,
    };
    let selected_workspace = Workspace::new(selected_workspace)
        .map_err(|error| RuntimeError::InvalidWorkspace(error.to_string()))?;
    user_config.workspace = selected_workspace.root().display().to_string();
    if let Some(command) = tunnel_override {
        user_config.tunnel_command = Some(config::parse_tunnel_command(command)?);
    }

    if let Some(existing) = read_state()? {
        if process_alive(existing.tunnel_pid) {
            return Ok(status_from_state(
                Some(user_config),
                existing,
                false,
                "already connected",
            ));
        }
        remove_state()?;
    }

    let workspace = selected_workspace;
    let argv = user_config
        .tunnel_command
        .as_ref()
        .ok_or(RuntimeError::TunnelNotConfigured)?;

    let state_dir = config::state_dir()?;
    fs::create_dir_all(&state_dir)?;
    let current_exe = std::env::current_exe()?;
    let server_args = json!([
        "serve",
        "--stdio",
        "--workspace",
        workspace.root().display().to_string()
    ]);

    let mut command = Command::new(&argv[0]);
    let shell_env = onboarding::managed_shell_env()
        .map_err(|error| RuntimeError::Io(std::io::Error::other(error.to_string())))?;
    let tunnel_client = onboarding::resolve_tunnel_client();
    command
        .args(&argv[1..])
        .env("WEB_HARNESS_SERVER_BIN", &current_exe)
        .env("WEB_HARNESS_SERVER_ARGS_JSON", server_args.to_string())
        .env("WEB_HARNESS_WORKSPACE", workspace.root())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(tunnel_client) = tunnel_client {
        command.env("WEB_HARNESS_TUNNEL_CLIENT_BIN", &tunnel_client);
        if let Some(parent) = tunnel_client.parent() {
            let mut paths = vec![parent.to_path_buf()];
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(path) = std::env::join_paths(paths) {
                command.env("PATH", path);
            }
        }
    }
    if let Some(tunnel_id) = shell_env.tunnel_id {
        command.env("CONTROL_PLANE_TUNNEL_ID", tunnel_id);
    }
    if let Some(api_key) = shell_env.api_key {
        command.env("CONTROL_PLANE_API_KEY", api_key);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = command.spawn()?;
    std::thread::sleep(std::time::Duration::from_millis(150));
    if !process_alive(child.id()) {
        return Err(RuntimeError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "tunnel process exited during startup; use tunnel doctor/accept for diagnostics",
        )));
    }

    let state = RuntimeState {
        schema_version: 1,
        tunnel_pid: child.id(),
        workspace: workspace.root().display().to_string(),
        started_unix_s: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    write_state(&state)?;
    Ok(status_from_state(
        Some(user_config),
        state,
        false,
        "connected",
    ))
}

pub fn status() -> Result<UserStatus, RuntimeError> {
    let user_config = config::load_optional()?;
    let Some(state) = read_state()? else {
        return Ok(UserStatus {
            configured: user_config.is_some(),
            connected: false,
            stale_state_recovered: false,
            workspace: user_config.as_ref().map(|value| value.workspace.clone()),
            tunnel_pid: None,
            sandbox: sandbox::status().into(),
            message: if user_config.is_some() {
                "configured but disconnected".into()
            } else {
                "not configured".into()
            },
        });
    };
    if !process_alive(state.tunnel_pid) {
        remove_state()?;
        return Ok(UserStatus {
            configured: user_config.is_some(),
            connected: false,
            stale_state_recovered: true,
            workspace: Some(state.workspace),
            tunnel_pid: None,
            sandbox: sandbox::status().into(),
            message: "stale connection state was cleaned up".into(),
        });
    }
    Ok(status_from_state(user_config, state, false, "connected"))
}

pub fn disconnect() -> Result<UserStatus, RuntimeError> {
    let user_config = config::load_optional()?;
    if let Some(state) = read_state()? {
        if process_alive(state.tunnel_pid) {
            terminate_process_group(state.tunnel_pid)?;
        }
        remove_state()?;
        return Ok(UserStatus {
            configured: user_config.is_some(),
            connected: false,
            stale_state_recovered: false,
            workspace: Some(state.workspace),
            tunnel_pid: None,
            sandbox: sandbox::status().into(),
            message: "disconnected".into(),
        });
    }
    Ok(UserStatus {
        configured: user_config.is_some(),
        connected: false,
        stale_state_recovered: false,
        workspace: user_config.as_ref().map(|value| value.workspace.clone()),
        tunnel_pid: None,
        sandbox: sandbox::status().into(),
        message: "already disconnected".into(),
    })
}

pub fn format_status(status: &UserStatus) -> String {
    format!(
        "web-harness\n  status: {}\n  workspace: {}\n  tunnel: {}\n  sandbox: {}\n  {}",
        if status.connected {
            "connected"
        } else {
            "disconnected"
        },
        status.workspace.as_deref().unwrap_or("-"),
        status
            .tunnel_pid
            .map(|pid| format!("running (pid {pid})"))
            .unwrap_or_else(|| "stopped".into()),
        status.sandbox,
        status.message
    )
}

fn status_from_state(
    user_config: Option<UserConfig>,
    state: RuntimeState,
    stale: bool,
    message: &str,
) -> UserStatus {
    UserStatus {
        configured: user_config.is_some(),
        connected: true,
        stale_state_recovered: stale,
        workspace: Some(state.workspace),
        tunnel_pid: Some(state.tunnel_pid),
        sandbox: sandbox::status().into(),
        message: message.into(),
    }
}

fn state_path() -> Result<PathBuf, RuntimeError> {
    Ok(config::state_dir()?.join("state.json"))
}

fn read_state() -> Result<Option<RuntimeState>, RuntimeError> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

fn write_state(state: &RuntimeState) -> Result<(), RuntimeError> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(state)?)?;
    fs::rename(temp, path)?;
    Ok(())
}

fn remove_state() -> Result<(), RuntimeError> {
    let path = state_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    unsafe {
        return libc::kill(pid as i32, 0) == 0;
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn terminate_process_group(pid: u32) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    unsafe {
        if libc::kill(-(pid as i32), libc::SIGTERM) == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        if process_alive(pid) {
            let _ = libc::kill(-(pid as i32), libc::SIGKILL);
        }
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_format_is_human_readable() {
        let value = UserStatus {
            configured: true,
            connected: false,
            stale_state_recovered: false,
            workspace: Some("/tmp/project".into()),
            tunnel_pid: None,
            sandbox: "test".into(),
            message: "disconnected".into(),
        };
        let output = format_status(&value);
        assert!(output.contains("workspace: /tmp/project"));
        assert!(output.contains("status: disconnected"));
    }
}
