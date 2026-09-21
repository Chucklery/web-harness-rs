use crate::command_output;
use crate::process;
use crate::redact;
use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

const OUTPUT_LIMIT: usize = 64 * 1024;
const EXTERNAL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum TunnelError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid tunnel command: {0}")]
    Invalid(String),
    #[error("local MCP acceptance failed: {0}")]
    Local(String),
}

#[derive(Debug, Serialize)]
pub struct TunnelReport {
    pub passed: bool,
    pub local_mcp_roundtrip: bool,
    pub external_command_configured: bool,
    pub external_command_passed: Option<bool>,
    pub external_exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub note: String,
}

pub fn doctor(workspace: &Workspace) -> Result<TunnelReport, TunnelError> {
    local_roundtrip(workspace)?;
    Ok(TunnelReport {
        passed: true,
        local_mcp_roundtrip: true,
        external_command_configured: false,
        external_command_passed: None,
        external_exit_code: None,
        stdout_tail: None,
        stderr_tail: None,
        note: "Local stdio MCP initialize/tools-list roundtrip passed. Configure an exact official tunnel acceptance command to test the remote tunnel.".into(),
    })
}

pub fn accept(
    workspace: &Workspace,
    command_json: Option<&str>,
) -> Result<TunnelReport, TunnelError> {
    let Some(command_json) = command_json else {
        local_roundtrip(workspace)?;
        return Ok(TunnelReport {
            passed: false,
            local_mcp_roundtrip: true,
            external_command_configured: false,
            external_command_passed: None,
            external_exit_code: None,
            stdout_tail: None,
            stderr_tail: None,
            note: "No external tunnel command configured. Set WEB_HARNESS_TUNNEL_COMMAND_JSON to a JSON argv array that performs the current official Secure MCP Tunnel acceptance flow.".into(),
        });
    };

    let argv = crate::config::parse_tunnel_command(command_json)
        .map_err(|error| TunnelError::Invalid(error.to_string()))?;
    local_roundtrip(workspace)?;

    let current_exe = std::env::current_exe()?;
    let server_args = json!([
        "serve",
        "--stdio",
        "--workspace",
        workspace.root().display().to_string()
    ]);
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .env("WEB_HARNESS_SERVER_BIN", &current_exe)
        .env("WEB_HARNESS_SERVER_ARGS_JSON", server_args.to_string())
        .env("WEB_HARNESS_WORKSPACE", workspace.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_in_group(command)?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TunnelError::Local("missing external stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| TunnelError::Local("missing external stderr".into()))?;
    let stdout_reader =
        std::thread::spawn(move || command_output::read_tail_stream(stdout, OUTPUT_LIMIT));
    let stderr_reader =
        std::thread::spawn(move || command_output::read_tail_stream(stderr, OUTPUT_LIMIT));

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= EXTERNAL_TIMEOUT {
            terminate_child(&mut child)?;
            let _ = child.wait();
            let stdout = stdout_reader
                .join()
                .unwrap_or_else(|_| Ok((Vec::new(), false)))?;
            let stderr = stderr_reader
                .join()
                .unwrap_or_else(|_| Ok((Vec::new(), false)))?;
            return Ok(TunnelReport {
                passed: false,
                local_mcp_roundtrip: true,
                external_command_configured: true,
                external_command_passed: Some(false),
                external_exit_code: None,
                stdout_tail: Some(redact::text(&String::from_utf8_lossy(&stdout.0))),
                stderr_tail: Some(format!(
                    "{}\nexternal tunnel acceptance command timed out after 120 seconds",
                    redact::text(&String::from_utf8_lossy(&stderr.0))
                )),
                note: "The injected command is responsible for exercising the current official Secure MCP Tunnel flow.".into(),
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let stdout = stdout_reader
        .join()
        .map_err(|_| TunnelError::Local("external stdout reader panicked".into()))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| TunnelError::Local("external stderr reader panicked".into()))??;
    let passed = status.success();
    Ok(TunnelReport {
        passed,
        local_mcp_roundtrip: true,
        external_command_configured: true,
        external_command_passed: Some(passed),
        external_exit_code: status.code(),
        stdout_tail: Some(redact::text(&String::from_utf8_lossy(&stdout.0))),
        stderr_tail: Some(redact::text(&String::from_utf8_lossy(&stderr.0))),
        note: "The external command is user/CI supplied so this project never invents tunnel-client flags. It should implement the current official Secure MCP Tunnel acceptance steps.".into(),
    })
}

fn terminate_child(child: &mut Child) -> Result<(), TunnelError> {
    if let Err(error) = process::terminate_process_group(child.id()) {
        if child.try_wait()?.is_none() {
            return Err(TunnelError::Local(error.to_string()));
        }
    }
    Ok(())
}

fn spawn_in_group(mut command: Command) -> Result<Child, TunnelError> {
    process::detach_into_own_group(&mut command)
        .map_err(|error| TunnelError::Local(error.to_string()))?;
    Ok(command.spawn()?)
}

fn local_roundtrip(workspace: &Workspace) -> Result<(), TunnelError> {
    let binary = std::env::current_exe()?;
    let workspace_arg = workspace.root().display().to_string();
    let mut command = Command::new(binary);
    command
        .args(["serve", "--stdio", "--workspace", &workspace_arg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_in_group(command)?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| TunnelError::Local("missing child stdin".into()))?;
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
    )?;
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )?;
    stdin.flush()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| TunnelError::Local("missing child stdout".into()))?;
    let mut lines = BufReader::new(stdout).lines();
    let initialize: Value = serde_json::from_str(
        &lines
            .next()
            .ok_or_else(|| TunnelError::Local("missing initialize response".into()))??,
    )?;
    if initialize["result"]["serverInfo"]["name"] != "web-harness" {
        return Err(TunnelError::Local("invalid initialize response".into()));
    }

    let tools: Value = serde_json::from_str(
        &lines
            .next()
            .ok_or_else(|| TunnelError::Local("missing tools/list response".into()))??,
    )?;
    if !tools["result"]["tools"].is_array() {
        return Err(TunnelError::Local("invalid tools/list response".into()));
    }

    drop(stdin);
    if !child.wait()?.success() {
        return Err(TunnelError::Local(
            "stdio server exited unsuccessfully".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounded_tail_keeps_the_end() {
        let (bytes, truncated) =
            command_output::read_tail_stream(Cursor::new(b"abcdef"), 3).unwrap();
        assert_eq!(bytes, b"def");
        assert!(truncated);
    }

    #[test]
    fn accept_rejects_secret_bearing_wrapper_args() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let error = accept(&workspace, Some(r#"["wrapper","--token=secret"]"#)).unwrap_err();
        assert!(error.to_string().contains("credentials"));
    }
}
