pub(crate) mod context;
pub(crate) mod exec;
pub(crate) mod file;
pub(crate) mod git;
pub(crate) mod job;
pub(crate) mod list_files;
pub(crate) mod manifest;
pub(crate) mod patch;
pub(crate) mod protected;
pub(crate) mod registry;
pub(crate) mod search;
pub(crate) mod status;
#[path = "runtime/trait.rs"]
pub(crate) mod tool_trait;
pub(crate) mod workspace;

pub(crate) use context::ExecutionContext;
pub(crate) use tool_trait::{RuntimeErrorKind, RuntimeToolError};

use crate::config::{self, UserConfig};
use crate::onboarding;
use crate::process;
use crate::sandbox;
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// How long the tunnel process is given to prove that it is still running after
/// it has been spawned.
const TUNNEL_STARTUP_GRACE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeState {
    schema_version: u32,
    tunnel_pid: u32,
    workspace: String,
    started_unix_s: u64,
    #[serde(default)]
    denied_paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UserStatus {
    pub configured: bool,
    pub connected: bool,
    pub stale_state_recovered: bool,
    pub workspace: Option<String>,
    /// Workspace-relative paths excluded from the boundary in effect.
    pub denied_paths: Vec<String>,
    pub tunnel_pid: Option<u32>,
    pub sandbox: String,
    pub message: String,
}

impl UserStatus {
    /// The single construction point for a status that carries no live tunnel.
    fn disconnected(
        configured: bool,
        stale_state_recovered: bool,
        workspace: Option<String>,
        message: &str,
    ) -> Self {
        Self {
            configured,
            connected: false,
            stale_state_recovered,
            workspace,
            denied_paths: Vec::new(),
            tunnel_pid: None,
            sandbox: sandbox::status().into(),
            message: message.into(),
        }
    }

    /// The single construction point for a status backed by a tunnel process.
    fn from_state(user_config: Option<UserConfig>, state: RuntimeState, message: &str) -> Self {
        Self {
            configured: user_config.is_some(),
            connected: true,
            stale_state_recovered: false,
            workspace: Some(state.workspace),
            denied_paths: Vec::new(),
            tunnel_pid: Some(state.tunnel_pid),
            sandbox: sandbox::status().into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Process(#[from] process::ProcessError),
    #[error("web-harness is not configured; run setup first")]
    NotConfigured,
    #[error("OpenAI tunnel-client was not found; reinstall web-harness or set WEB_HARNESS_TUNNEL_CLIENT_BIN")]
    TunnelClientUnavailable,
    #[error("configured workspace is invalid: {0}")]
    InvalidWorkspace(String),
}

pub fn connect(
    workspace_override: Option<&Path>,
    tunnel_override: Option<&str>,
    deny_paths: Option<&str>,
) -> Result<UserStatus, RuntimeError> {
    let mut user_config = config::load_optional()?.ok_or(RuntimeError::NotConfigured)?;
    let selected_workspace = match workspace_override {
        Some(workspace) => workspace.to_path_buf(),
        None => std::env::current_dir()?,
    };
    let denied = crate::workspace::effective_deny_paths(deny_paths)
        .map_err(|error| RuntimeError::InvalidWorkspace(error.to_string()))?;
    let selected_workspace = Workspace::with_denied(selected_workspace, &denied)
        .map_err(|error| RuntimeError::InvalidWorkspace(error.to_string()))?;
    user_config.workspace = selected_workspace.root().display().to_string();
    if let Some(command) = tunnel_override {
        user_config.tunnel_command = Some(config::parse_tunnel_command(command)?);
    }

    if let Some(existing) = read_state()? {
        if process::process_alive(existing.tunnel_pid) {
            let existing_denied = if existing.denied_paths.is_empty() {
                denied.clone()
            } else {
                existing.denied_paths.clone()
            };
            return Ok(connected_status(
                Some(user_config),
                existing,
                existing_denied,
                "already connected",
            ));
        }
        remove_state()?;
    }

    let workspace = selected_workspace;

    let state_dir = config::state_dir()?;
    fs::create_dir_all(&state_dir)?;
    let current_exe = std::env::current_exe()?;
    let server_args = json!([
        "serve",
        "--stdio",
        "--workspace",
        workspace.root().display().to_string(),
        "--deny-paths",
        workspace.denied().join(",")
    ]);

    let shell_env = onboarding::managed_shell_env()
        .map_err(|error| RuntimeError::Io(std::io::Error::other(error.to_string())))?;
    let tunnel_client = onboarding::resolve_tunnel_client();
    let mut command = if let Some(argv) = user_config.tunnel_command.as_ref() {
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        command
    } else {
        let tunnel_client = tunnel_client
            .as_ref()
            .ok_or(RuntimeError::TunnelClientUnavailable)?;
        let mcp_command =
            build_stdio_mcp_command(&current_exe, workspace.root(), &workspace.denied());
        let mut command = Command::new(tunnel_client);
        command
            .arg("run")
            .arg(format!("--mcp.command={mcp_command}"));
        command
    };
    command
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
    for (key, value) in [
        ("CONTROL_PLANE_TUNNEL_ID", shell_env.tunnel_id),
        ("CONTROL_PLANE_API_KEY", shell_env.api_key),
    ] {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    process::detach_into_own_group(&mut command)?;
    let child = command.spawn()?;
    std::thread::sleep(TUNNEL_STARTUP_GRACE);
    if !process::process_alive(child.id()) {
        return Err(RuntimeError::Io(std::io::Error::other(
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
        denied_paths: workspace.denied(),
    };
    write_state(&state)?;
    Ok(connected_status(
        Some(user_config),
        state,
        workspace.denied(),
        "connected",
    ))
}

pub fn status() -> Result<UserStatus, RuntimeError> {
    let user_config = config::load_optional()?;
    let configured = user_config.is_some();
    let denied = match user_config.as_ref() {
        Some(config) => {
            let workspace = Path::new(&config.workspace);
            if workspace.is_dir() {
                crate::workspace::effective_deny_paths(None)
                    .ok()
                    .and_then(|denied| Workspace::with_denied(workspace, &denied).ok())
                    .map(|value| value.denied())
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    };
    let Some(state) = read_state()? else {
        let mut status = UserStatus::disconnected(
            configured,
            false,
            user_config.as_ref().map(|value| value.workspace.clone()),
            if configured {
                "configured but disconnected"
            } else {
                "not configured"
            },
        );
        status.denied_paths = denied;
        return Ok(status);
    };
    if !process::process_alive(state.tunnel_pid) {
        remove_state()?;
        let mut status = UserStatus::disconnected(
            configured,
            true,
            Some(state.workspace),
            "stale connection state was cleaned up",
        );
        status.denied_paths = denied;
        return Ok(status);
    }
    let state_denied = if state.denied_paths.is_empty() {
        denied
    } else {
        state.denied_paths.clone()
    };
    Ok(connected_status(
        user_config,
        state,
        state_denied,
        "connected",
    ))
}

fn connected_status(
    user_config: Option<UserConfig>,
    state: RuntimeState,
    denied_paths: Vec<String>,
    message: &str,
) -> UserStatus {
    let mut status = UserStatus::from_state(user_config, state, message);
    status.denied_paths = denied_paths;
    status
}

pub fn disconnect() -> Result<UserStatus, RuntimeError> {
    let user_config = config::load_optional()?;
    let configured = user_config.is_some();
    let Some(state) = read_state()? else {
        return Ok(UserStatus::disconnected(
            configured,
            false,
            user_config.as_ref().map(|value| value.workspace.clone()),
            "already disconnected",
        ));
    };
    if process::process_alive(state.tunnel_pid) {
        process::terminate_process_group(state.tunnel_pid)?;
    }
    remove_state()?;
    Ok(UserStatus::disconnected(
        configured,
        false,
        Some(state.workspace),
        "disconnected",
    ))
}

pub fn format_status(status: &UserStatus) -> String {
    let boundary = if status.denied_paths.is_empty() {
        "none".to_string()
    } else {
        status.denied_paths.join(", ")
    };
    format!(
        "web-harness\n  status: {}\n  workspace: {}\n  excluded: {}\n  tunnel: {}\n  sandbox: {}\n  {}",
        if status.connected {
            "connected"
        } else {
            "disconnected"
        },
        status.workspace.as_deref().unwrap_or("-"),
        boundary,
        status
            .tunnel_pid
            .map(|pid| format!("running (pid {pid})"))
            .unwrap_or_else(|| "stopped".into()),
        status.sandbox,
        status.message
    )
}

/// Multi-line confirmation printed by `connect`, so the boundary in effect is
/// explicit before any remote agent can reach the workspace.
pub fn format_connect_confirmation(status: &UserStatus) -> String {
    let boundary = if status.denied_paths.is_empty() {
        "none".to_string()
    } else {
        status.denied_paths.join(", ")
    };
    format!(
        "web-harness\n  workspace: {}\n  excluded: {}\n  sandbox: {}\n  tunnel: {}\n  {}",
        status.workspace.as_deref().unwrap_or("-"),
        boundary,
        status.sandbox,
        status
            .tunnel_pid
            .map(|pid| format!("running (pid {pid})"))
            .unwrap_or_else(|| "stopped".into()),
        status.message
    )
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

fn build_stdio_mcp_command(executable: &Path, workspace: &Path, denied: &[String]) -> String {
    format!(
        "{} serve --stdio --workspace {} --deny-paths {}",
        quote_command_arg(executable),
        quote_command_arg(workspace),
        quote_command_arg(Path::new(&denied.join(",")))
    )
}

fn quote_command_arg(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disconnected_status(workspace: Option<&str>, message: &str) -> UserStatus {
        UserStatus::disconnected(true, false, workspace.map(ToOwned::to_owned), message)
    }

    #[test]
    fn status_format_is_human_readable() {
        let value = disconnected_status(Some("/tmp/project"), "disconnected");
        let output = format_status(&value);
        assert!(output.contains("workspace: /tmp/project"));
        assert!(output.contains("status: disconnected"));
    }

    #[test]
    fn disconnected_status_reports_the_sandbox_backend() {
        let value = disconnected_status(None, "not configured");
        assert!(!value.connected);
        assert!(value.workspace.is_none());
        assert_eq!(value.sandbox, sandbox::status());
    }

    #[test]
    fn connected_status_preserves_the_workspace_boundary() {
        let value = connected_status(
            None,
            RuntimeState {
                schema_version: 1,
                tunnel_pid: std::process::id(),
                workspace: "/tmp/project".into(),
                started_unix_s: 0,
                denied_paths: Vec::new(),
            },
            vec![".git".into(), "target".into()],
            "connected",
        );
        assert_eq!(value.denied_paths, [".git", "target"]);
        assert!(format_status(&value).contains("excluded: .git, target"));
    }

    #[test]
    fn runtime_state_keeps_legacy_files_readable() {
        let value: RuntimeState = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "tunnel_pid": 42,
            "workspace": "/tmp/project",
            "started_unix_s": 0
        }))
        .unwrap();
        assert!(value.denied_paths.is_empty());
    }

    #[test]
    fn stdio_command_quotes_paths() {
        let denied = vec![".git".to_string(), "target".to_string()];
        let command = build_stdio_mcp_command(
            Path::new("/tmp/web harness"),
            Path::new("/tmp/project with spaces"),
            &denied,
        );
        assert!(command.contains("serve --stdio --workspace"));
        assert!(command.contains("web harness"));
        assert!(command.contains("project with spaces"));
        assert!(command.contains("--deny-paths '.git,target'"));
    }
}
