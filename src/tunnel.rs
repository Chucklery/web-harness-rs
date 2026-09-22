use crate::command_output;
use crate::process;
use crate::redact;
use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Lines, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
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
    local_fixture_roundtrip()?;
    Ok(TunnelReport {
        passed: true,
        local_mcp_roundtrip: true,
        external_command_configured: false,
        external_command_passed: None,
        external_exit_code: None,
        stdout_tail: None,
        stderr_tail: None,
        note: "Local stdio MCP initialize/tools-list, bounded read-only calls, and isolated write/exec/job/Git roundtrips passed. Configure an exact official tunnel acceptance command to test the remote tunnel.".into(),
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
                note: "Local stdio MCP checks passed, but no external tunnel command is configured. Set WEB_HARNESS_TUNNEL_COMMAND_JSON to a JSON argv array that performs the current official Secure MCP Tunnel acceptance flow.".into(),
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
    let mut child = ChildGuard::new(spawn_in_group(command)?);

    let stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| TunnelError::Local("missing external stdout".into()))?;
    let stderr = child
        .child_mut()
        .stderr
        .take()
        .ok_or_else(|| TunnelError::Local("missing external stderr".into()))?;
    let stdout_reader =
        std::thread::spawn(move || command_output::read_tail_stream(stdout, OUTPUT_LIMIT));
    let stderr_reader =
        std::thread::spawn(move || command_output::read_tail_stream(stderr, OUTPUT_LIMIT));

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.child_mut().try_wait()? {
            break status;
        }
        if start.elapsed() >= EXTERNAL_TIMEOUT {
            terminate_child(child.child_mut())?;
            let _ = child.child_mut().wait();
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
    child.disarm();
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

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child guard must be armed")
    }

    fn disarm(mut self) {
        self.child.take();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let _ = process::terminate_process_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn_in_group(mut command: Command) -> Result<Child, TunnelError> {
    process::detach_into_own_group(&mut command)
        .map_err(|error| TunnelError::Local(error.to_string()))?;
    Ok(command.spawn()?)
}

struct LocalMcpClient {
    child: ChildGuard,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl LocalMcpClient {
    fn start(workspace: &Path) -> Result<Self, TunnelError> {
        let binary = std::env::current_exe()?;
        let workspace_arg = workspace.display().to_string();
        let mut command = Command::new(binary);
        command
            .args(["serve", "--stdio", "--workspace", &workspace_arg])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = ChildGuard::new(spawn_in_group(command)?);
        let stdin = child
            .child_mut()
            .stdin
            .take()
            .ok_or_else(|| TunnelError::Local("missing fixture stdin".into()))?;
        let stdout = child
            .child_mut()
            .stdout
            .take()
            .ok_or_else(|| TunnelError::Local("missing fixture stdout".into()))?;
        Ok(Self {
            child,
            stdin,
            lines: BufReader::new(stdout).lines(),
            next_id: 1,
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, TunnelError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )?;
        self.stdin.flush()?;
        loop {
            let value: Value = serde_json::from_str(
                &self
                    .lines
                    .next()
                    .ok_or_else(|| TunnelError::Local("fixture server closed stdout".into()))??,
            )?;
            if value["method"] == "elicitation/create" {
                writeln!(
                    self.stdin,
                    "{}",
                    json!({
                        "jsonrpc":"2.0",
                        "id":value["id"],
                        "result":{"action":"accept","content":{"approved":true}}
                    })
                )?;
                self.stdin.flush()?;
                continue;
            }
            if value["id"] == id {
                return Ok(value);
            }
        }
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, TunnelError> {
        let response = self.request("tools/call", json!({"name":name,"arguments":arguments}))?;
        if response.get("error").is_some() {
            return Err(TunnelError::Local(format!(
                "fixture tool {name} returned a protocol error"
            )));
        }
        if response["result"]["isError"] == true {
            return Err(TunnelError::Local(format!(
                "fixture tool {name} returned a tool error"
            )));
        }
        Ok(response["result"]
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| response["result"].clone()))
    }

    fn finish(mut self) -> Result<(), TunnelError> {
        drop(self.stdin);
        let status = self.child.child_mut().wait()?;
        self.child.disarm();
        if status.success() {
            Ok(())
        } else {
            Err(TunnelError::Local(
                "fixture server exited unsuccessfully".into(),
            ))
        }
    }
}

struct FixtureDirectory {
    path: PathBuf,
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn local_fixture_roundtrip() -> Result<(), TunnelError> {
    let path = std::env::temp_dir().join(format!(
        "web-harness-doctor-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&path)?;
    let fixture = FixtureDirectory { path };
    fs::write(fixture.path.join("AGENTS.md"), "Use bounded tools.\n")?;
    fs::write(fixture.path.join("hello.txt"), "needle\n")?;
    let git_status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&fixture.path)
        .status()?;
    if !git_status.success() {
        return Err(TunnelError::Local(
            "failed to initialize fixture Git repository".into(),
        ));
    }

    let mut client = LocalMcpClient::start(&fixture.path)?;
    let initialized = client.request(
        "initialize",
        json!({"capabilities":{"elicitation":{"form":{}}}}),
    )?;
    if initialized["result"]["serverInfo"]["name"] != "web-harness"
        || initialized["result"]["protocolVersion"] != "2025-06-18"
    {
        return Err(TunnelError::Local(
            "invalid fixture initialize response".into(),
        ));
    }
    let tools = client.request("tools/list", json!({}))?;
    if !tools["result"]["tools"]
        .as_array()
        .is_some_and(|tools| tools.len() >= 9)
    {
        return Err(TunnelError::Local(
            "invalid fixture tools/list response".into(),
        ));
    }
    let recovered = client.request(
        "tools/call",
        json!({"name":"read_files","arguments":{"paths":["missing-fixture.txt"]}}),
    )?;
    if recovered["result"]["isError"] != true
        || recovered["result"]["structuredContent"]["error"]["code"].is_null()
    {
        return Err(TunnelError::Local(
            "fixture error response was not structured or recoverable".into(),
        ));
    }
    client.call_tool("workspace_info", json!({}))?;
    client.call_tool("workspace_instructions", json!({"path":"."}))?;
    client.call_tool("list_files", json!({"source":"all","limit":20}))?;
    let search = client.call_tool(
        "search",
        json!({"query":"needle","literal":true,"max_results":10}),
    )?;
    if search["matches"].as_array().is_none() {
        return Err(TunnelError::Local(
            "fixture search result was invalid".into(),
        ));
    }
    client.call_tool("read_files", json!({"paths":["hello.txt"]}))?;
    client.call_tool(
        "patch",
        json!({"patch":"*** Begin Patch\n*** Add File: nested/new.txt\n+created\n*** End Patch"}),
    )?;
    client.call_tool("read_files", json!({"paths":["nested/new.txt"]}))?;
    client.call_tool("exec", json!({"argv":["git","status","--short"]}))?;
    let job = client.call_tool(
        "exec",
        json!({"argv":["git","status","--short"],"background":true}),
    )?;
    let job_id = job["id"]
        .as_str()
        .ok_or_else(|| TunnelError::Local("fixture background job id missing".into()))?;
    client.call_tool(
        "job",
        json!({"action":"wait","id":job_id,"timeout_ms":5000}),
    )?;
    client.call_tool(
        "job",
        json!({"action":"output","id":job_id,"stream":"stdout"}),
    )?;
    client.call_tool("git", json!({"action":"status"}))?;
    client.call_tool("git", json!({"action":"add","pathspec":["nested/new.txt"]}))?;
    client.finish()
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
    let mut child = ChildGuard::new(spawn_in_group(command)?);

    let mut stdin = child
        .child_mut()
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
        .child_mut()
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

    for (id, params) in [
        (3, json!({"name":"workspace_info","arguments":{}})),
        (4, json!({"name":"list_files","arguments":{"limit":1}})),
        (
            5,
            json!({"name":"search","arguments":{"query":"web-harness","literal":true,"max_results":1}}),
        ),
        (6, json!({"name":"git","arguments":{"action":"status"}})),
    ] {
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params})
        )?;
    }
    stdin.flush()?;
    for expected_id in 3..=6 {
        let response: Value = serde_json::from_str(
            &lines
                .next()
                .ok_or_else(|| TunnelError::Local("missing tools/call response".into()))??,
        )?;
        if response["id"] != expected_id || !response["result"]["content"].is_array() {
            return Err(TunnelError::Local(
                "invalid tools/call result envelope".into(),
            ));
        }
    }

    drop(stdin);
    if !child.child_mut().wait()?.success() {
        return Err(TunnelError::Local(
            "stdio server exited unsuccessfully".into(),
        ));
    }
    child.disarm();
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
