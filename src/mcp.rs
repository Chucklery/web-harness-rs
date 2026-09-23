use crate::jobs::JobManager;
use crate::permission::PermissionEngine;
use crate::runtime::{
    manifest, registry::RuntimeRegistry, status, ExecutionContext, RuntimeErrorKind,
    RuntimeToolError,
};
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const MAX_PROTOCOL_LINE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
struct Request {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Default)]
struct Session {
    next_server_request_id: u64,
    elicitation_supported: bool,
}

#[derive(Debug, Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

pub fn serve_stdio(workspace: Workspace) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout().lock();
    let mut jobs = JobManager::new();
    let mut permissions = PermissionEngine::new()?;
    let runtime = RuntimeRegistry::default();
    let mut session = Session {
        next_server_request_id: 1,
        ..Session::default()
    };
    loop {
        let line = match read_bounded_line(&mut stdin) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                let response = Response {
                    jsonrpc: "2.0",
                    id: None,
                    result: None,
                    error: Some(json!({"code": -32700, "message": error.to_string()})),
                };
                serde_json::to_writer(&mut stdout, &response)?;
                writeln!(&mut stdout)?;
                stdout.flush()?;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(request) if request.id.is_none() && request.method.starts_with("notifications/") => {
                continue;
            }
            Ok(request) => request,
            Err(error) => {
                let response = Response {
                    jsonrpc: "2.0",
                    id: None,
                    result: None,
                    error: Some(json!({"code": -32700, "message": error.to_string()})),
                };
                serde_json::to_writer(&mut stdout, &response)?;
                writeln!(&mut stdout)?;
                stdout.flush()?;
                continue;
            }
        };
        if request.method == "initialize" {
            session.elicitation_supported = request
                .params
                .get("capabilities")
                .and_then(|capabilities| capabilities.get("elicitation"))
                .is_some_and(Value::is_object);
        }
        let mut response = handle(
            request.clone(),
            &workspace,
            &mut jobs,
            &mut permissions,
            &runtime,
        );
        let mut retry = request.clone();
        while session.elicitation_supported
            && request.method == "tools/call"
            && is_approval_required(&response)
        {
            if let Some(approval_id) = response
                .result
                .as_ref()
                .and_then(|result| result.get("structuredContent"))
                .and_then(|value| value.get("approval"))
                .and_then(|approval| approval.get("id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
            {
                let approval_argument = response
                    .result
                    .as_ref()
                    .and_then(|result| result.get("structuredContent"))
                    .and_then(|value| value.get("approval_argument"))
                    .and_then(Value::as_str)
                    .unwrap_or("approval_id")
                    .to_string();
                match request_host_approval(&mut stdin, &mut stdout, &mut session, &response)? {
                    Some(true) => {
                        permissions.approve(&approval_id)?;
                        let arguments = retry
                            .params
                            .get_mut("arguments")
                            .and_then(Value::as_object_mut)
                            .ok_or("tools/call arguments must be an object")?;
                        arguments.insert(approval_argument, Value::String(approval_id));
                        response = handle(
                            retry.clone(),
                            &workspace,
                            &mut jobs,
                            &mut permissions,
                            &runtime,
                        );
                    }
                    Some(false) => {
                        permissions.deny(&approval_id)?;
                        response = denied_response(&response);
                        break;
                    }
                    None => break,
                }
            } else {
                break;
            }
        }
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(&mut stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

fn is_approval_required(response: &Response) -> bool {
    response
        .result
        .as_ref()
        .and_then(|result| result.get("structuredContent"))
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("approval_required")
}

fn request_host_approval<R: BufRead, W: Write>(
    stdin: &mut R,
    stdout: &mut W,
    session: &mut Session,
    response: &Response,
) -> Result<Option<bool>, Box<dyn std::error::Error>> {
    let structured = response
        .result
        .as_ref()
        .and_then(|result| result.get("structuredContent"))
        .ok_or("approval response is missing structured content")?;
    let message = structured
        .get("approval")
        .and_then(|approval| approval.get("summary"))
        .and_then(Value::as_str)
        .unwrap_or("Confirm this protected operation");
    let request_id = format!("web_harness_elicitation_{}", session.next_server_request_id);
    session.next_server_request_id = session.next_server_request_id.saturating_add(1);
    let request = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "elicitation/create",
        "params": {
            "mode": "form",
            "message": message,
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "approved": {"type": "boolean", "description": "Approve this one-time operation"}
                },
                "required": ["approved"],
                "additionalProperties": false
            }
        }
    });
    serde_json::to_writer(&mut *stdout, &request)?;
    writeln!(stdout)?;
    stdout.flush()?;

    loop {
        let line = match read_bounded_line(stdin) {
            Ok(Some(line)) => line,
            Ok(None) => return Ok(None),
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("id") != Some(&json!(request_id)) {
            continue;
        }
        let Some(result) = value.get("result") else {
            return Ok(Some(false));
        };
        if result.get("action").and_then(Value::as_str) == Some("accept") {
            return Ok(Some(
                result
                    .get("content")
                    .and_then(|content| content.get("approved"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ));
        }
        return Ok(Some(false));
    }
}

/// Read one UTF-8 protocol line while keeping both the retained bytes and the
/// recovery path bounded. If a line is too large, consume its remainder before
/// returning an error so the next request can still be processed.
fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let mut oversized = false;

    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if bytes.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(buffer.len());
        if bytes.len().saturating_add(content_len) > MAX_PROTOCOL_LINE_BYTES {
            oversized = true;
        } else if !oversized {
            bytes.extend_from_slice(&buffer[..content_len]);
        }

        let consumed = newline.map_or(buffer.len(), |index| index + 1);
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }

    if oversized {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("MCP protocol line exceeds {MAX_PROTOCOL_LINE_BYTES} bytes"),
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn denied_response(response: &Response) -> Response {
    let capability = response
        .result
        .as_ref()
        .and_then(|result| result.get("structuredContent"))
        .and_then(|value| value.get("capability"))
        .cloned()
        .unwrap_or(Value::Null);
    Response {
        jsonrpc: "2.0",
        id: response.id.clone(),
        result: Some(runtime_content(json!({
            "status": "denied",
            "capability": capability
        }))),
        error: None,
    }
}

fn handle(
    request: Request,
    workspace: &Workspace,
    jobs: &mut JobManager,
    permissions: &mut PermissionEngine,
    runtime: &RuntimeRegistry,
) -> Response {
    if request.jsonrpc != "2.0" {
        return Response {
            jsonrpc: "2.0",
            id: request.id,
            result: None,
            error: Some(json!({"code": -32600, "message": "jsonrpc must be 2.0"})),
        };
    }
    let result = match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "web-harness", "version": env!("CARGO_PKG_VERSION")}
        })),
        "tools/list" => {
            let mut value = json!({"tools": [
                {
                    "name": "workspace_info",
                    "description": "Return the configured workspace root.",
                    "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
                },
                {
                    "name": "read_files",
                    "description": "Read UTF-8 files inside the workspace with bounded size.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"paths": {"type": "array", "items": {"oneOf": [{"type": "string"}, {"type": "object", "properties": {"path": {"type": "string"}, "start_line": {"type": "integer", "minimum": 1}, "end_line": {"type": "integer", "minimum": 1}, "expected_read_revision": {"type": "string", "maxLength": 128}}, "required": ["path"], "additionalProperties": false}]}, "minItems": 1, "maxItems": 16}, "approval_id": {"type": "string"}},
                        "required": ["paths"],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "list_files",
                    "description": "List bounded workspace files and directories without executing a shell command.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {"type": "string"},
                            "source": {"type": "string", "enum": ["tracked", "all"], "default": "tracked"},
                            "offset": {"type": "integer", "minimum": 0},
                            "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 100},
                            "include": {"type": "string", "maxLength": 256}
                        },
                        "additionalProperties": false
                    }
                },
                {
                    "name": "search",
                    "description": "Search workspace content using ripgrep with bounded results.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string", "minLength": 1, "maxLength": 1024},
                            "queries": {
                                "type": "array",
                                "items": {"type": "string", "minLength": 1, "maxLength": 1024},
                                "minItems": 1,
                                "maxItems": 8
                            },
                            "max_results": {"type": "integer", "minimum": 1, "maximum": 200}
                            ,"literal": {"type": "boolean", "default": false},
                            "offset": {"type": "integer", "minimum": 0, "maximum": 100000},
                            "approval_id": {"type": "string"},
                            "scope": {"type": "string"},
                            "include": {"type": "array", "items": {"type": "string", "maxLength": 256}, "maxItems": 16},
                            "exclude": {"type": "array", "items": {"type": "string", "maxLength": 256}, "maxItems": 16},
                            "mode": {"type": "string", "enum": ["matches", "files_with_matches", "count"], "default": "matches"}
                        },
                        "oneOf": [
                            {"required": ["query"], "not": {"required": ["queries"]}},
                            {"required": ["queries"], "not": {"required": ["query"]}}
                        ],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "workspace_instructions",
                    "description": "Return scoped AGENTS.md instructions for a workspace-relative path.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"path": {"type": "string"}},
                        "additionalProperties": false
                    }
                },
                {
                    "name": "patch",
                    "description": "Apply a bounded Codex-style structured patch inside the workspace, returning changed paths and per-file added/removed byte counts.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "patch": {"type": "string", "maxLength": 524288},
                            "expected_read_revisions": {"type": "object", "maxProperties": 32, "additionalProperties": {"type": "string", "maxLength": 128}}
                        },
                        "required": ["patch"],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "exec",
                    "description": "Execute a bounded argv command or explicitly approved one-shot script in the workspace.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 64},
                            "script": {"type": "string", "maxLength": 65536},
                            "shell": {"type": "string", "enum": ["sh", "bash", "powershell", "pwsh"]},
                            "cwd": {"type": "string"},
                            "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 600000},
                            "stdin": {"type": "string", "maxLength": 65536},
                            "background": {"type": "boolean"},
                            "network": {"type": "string", "enum": ["deny", "outbound"], "default": "deny"},
                            "approval_id": {"type": "string"},
                            "network_approval_id": {"type": "string"}
                        },
                        "oneOf": [{"required": ["argv"]}, {"required": ["script"]}],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "job",
                    "description": "Poll, wait for, read output from, list, or cancel a background job. wait returns incremental stdout and stderr together.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {"type": "string", "enum": ["poll", "wait", "output", "cancel", "list"]},
                            "id": {"type": "string"},
                            "stream": {"type": "string", "enum": ["stdout", "stderr"]},
                            "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 60000},
                            "cursor": {"type": "integer", "minimum": 0},
                            "stdout_cursor": {"type": "integer", "minimum": 0},
                            "stderr_cursor": {"type": "integer", "minimum": 0}
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "git",
                    "description": "Run structured Git operations. Mutating actions require one-time approval.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {"type": "string", "enum": ["status", "diff", "log", "show", "show_file", "add", "commit", "switch", "create_branch", "restore", "push"]},
                            "staged": {"type": "boolean"},
                            "pathspec": {"type": "array", "items": {"type": "string"}, "maxItems": 32},
                            "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                            "offset": {"type": "integer", "minimum": 0, "maximum": 100000},
                            "revision": {"type": "string", "maxLength": 256},
                            "start_line": {"type": "integer", "minimum": 1, "maximum": 1000000},
                            "max_bytes": {"type": "integer", "minimum": 1, "maximum": 262144},
                            "message": {"type": "string", "minLength": 1, "maxLength": 4096},
                            "branch": {"type": "string", "minLength": 1, "maxLength": 256},
                            "remote": {"type": "string", "minLength": 1, "maxLength": 256},
                            "refspec": {"type": "string", "minLength": 1, "maxLength": 256},
                            "expected_head": {"type": "string", "minLength": 1, "maxLength": 256},
                            "approval_id": {"type": "string"}
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                },
                {
                    "name": "runtime_status",
                    "description": "Return compact status for the local web-harness adaptive runtime shim.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "compact": {"type": "boolean"},
                            "client_id": {"type": "string"}
                        },
                        "additionalProperties": true
                    }
                },
                {
                    "name": "work_on_project",
                    "description": "Bind work to the already-configured local workspace without creating another agent runtime.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project": {"type": "string"},
                            "client_id": {"type": "string"},
                            "path": {"type": "string"},
                            "instruction": {"type": "string", "maxLength": 4000},
                            "session_id": {"type": "string"}
                        },
                        "additionalProperties": true
                    }
                },
                {
                    "name": "tool_manifest",
                    "description": "Describe the bounded runtime tools that call_runtime_tool may invoke.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tool_name": {"type": "string"},
                            "category": {"type": "string"},
                            "intent": {"type": "string"},
                            "include_recommended_flows": {"type": "boolean"}
                        },
                        "additionalProperties": true
                    }
                },
                {
                    "name": "call_runtime_tool",
                    "description": "Invoke one existing bounded web-harness runtime tool through the compatibility gateway.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tool": {"type": "string"},
                            "arguments": {"type": "object"}
                        },
                        "required": ["tool"],
                        "additionalProperties": true
                    }
                }
            ]});
            annotate_tools(&mut value);
            Ok(value)
        }
        "tools/call" => call_tool(&request.params, workspace, jobs, permissions, runtime),
        method => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
    };
    match result {
        Ok(value) => Response {
            jsonrpc: "2.0",
            id: request.id,
            result: Some(value),
            error: None,
        },
        Err(error) => Response {
            jsonrpc: "2.0",
            id: request.id,
            result: None,
            error: Some(error),
        },
    }
}

fn runtime_content(value: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": value.to_string()}],
        "structuredContent": value
    })
}

fn annotate_tools(value: &mut Value) {
    let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str) else {
            continue;
        };
        let (read_only, destructive, open_world) = match name {
            "workspace_info"
            | "read_files"
            | "list_files"
            | "search"
            | "workspace_instructions"
            | "runtime_status"
            | "work_on_project"
            | "tool_manifest" => (true, false, false),
            "patch" | "job" => (false, true, false),
            "exec" | "git" | "call_runtime_tool" => (false, true, true),
            _ => (false, true, true),
        };
        if let Some(object) = tool.as_object_mut() {
            object.insert(
                "annotations".into(),
                json!({
                    "readOnlyHint": read_only,
                    "destructiveHint": destructive,
                    "openWorldHint": open_world
                }),
            );
        }
    }
}

fn runtime_error(tool: &str, error: RuntimeToolError) -> Value {
    let kind = error.kind();
    let code = match kind {
        RuntimeErrorKind::InvalidArguments => -32602,
        RuntimeErrorKind::NotFound => -32004,
        RuntimeErrorKind::Denied => -32007,
        RuntimeErrorKind::NotRegularFile => -32008,
        RuntimeErrorKind::InvalidEncoding => -32005,
        RuntimeErrorKind::Range => -32006,
        RuntimeErrorKind::Workspace => match tool {
            "workspace_instructions" => -32011,
            _ => -32001,
        },
        RuntimeErrorKind::LimitExceeded => match tool {
            "patch" => -32020,
            _ => -32002,
        },
        RuntimeErrorKind::Execution => match tool {
            "search" => -32010,
            "exec" => -32030,
            "job" => -32031,
            "git" => -32040,
            "patch" => -32021,
            _ => -32603,
        },
        RuntimeErrorKind::Permission => match tool {
            "exec" => -32032,
            "git" => -32041,
            "patch" => -32022,
            _ => -32033,
        },
        RuntimeErrorKind::Conflict => match tool {
            "read_files" => -32023,
            "patch" => -32024,
            _ => -32003,
        },
        // Distinct from Execution so a client can tell "the environment cannot
        // serve this" from "this request failed", without parsing prose.
        RuntimeErrorKind::Dependency => match tool {
            "search" => -32012,
            "git" => -32042,
            _ => -32012,
        },
    };
    let mut payload = json!({"code": code, "message": error.message()});
    if kind == RuntimeErrorKind::Dependency {
        payload["reason_code"] = json!("dependency_unavailable");
        payload["failure_stage"] = json!("dependency_resolution");
        payload["retryable"] = json!(false);
    }
    payload
}

fn runtime_tool_error(tool: &str, error: RuntimeToolError) -> Value {
    let payload = runtime_error(tool, error);
    json!({
        "content": [{"type": "text", "text": payload.to_string()}],
        "structuredContent": {"error": payload},
        "isError": true
    })
}

fn call_tool(
    params: &Value,
    workspace: &Workspace,
    jobs: &mut JobManager,
    permissions: &mut PermissionEngine,
    runtime: &RuntimeRegistry,
) -> Result<Value, Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| json!({"code": -32602, "message": "missing tool name"}))?;
    let runtime_result = {
        let mut context = ExecutionContext::new(workspace, jobs, permissions);
        runtime.call(
            name,
            &mut context,
            params.get("arguments").unwrap_or(&Value::Null),
        )
    };
    if let Some(result) = runtime_result {
        return Ok(match result {
            Ok(value) => runtime_content(value),
            Err(error) => runtime_tool_error(name, error),
        });
    }
    match name {
        "runtime_status" => {
            let context = ExecutionContext::new(workspace, jobs, permissions);
            Ok(runtime_content(status::describe(runtime, &context)))
        }
        "work_on_project" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            if let Some(path) = arguments.get("path").and_then(Value::as_str) {
                let requested = std::fs::canonicalize(path)
                    .map_err(|error| json!({"code": -32050, "message": error.to_string()}))?;
                if requested != workspace.root() {
                    return Err(json!({
                        "code": -32051,
                        "message": "web-harness is bound to one configured workspace; requested path does not match it"
                    }));
                }
            }
            let session_id = arguments
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("wc_sess_local_{}", std::process::id()));
            let value = json!({
                "session_id": session_id,
                "project": "web-harness:workspace",
                "resolved_project": "web-harness:workspace",
                "continuation": "configured_workspace",
                "workspace": {
                    "status": "ready",
                    "root": workspace.root().display().to_string()
                },
                "readiness": {"status": "ready"},
                "runtime": "adaptive_shim"
            });
            Ok(runtime_content(value))
        }
        "tool_manifest" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let requested = arguments.get("tool_name").and_then(Value::as_str);
            manifest::describe(runtime, requested)
                .map(runtime_content)
                .map_err(|error| runtime_error("tool_manifest", error))
        }
        "call_runtime_tool" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let tool = arguments
                .get("tool")
                .and_then(Value::as_str)
                .ok_or_else(|| json!({"code": -32602, "message": "arguments.tool is required"}))?;
            if !runtime.contains(tool) {
                return Err(json!({
                    "code": -32602,
                    "message": format!("runtime tool is not exposed through the compatibility gateway: {tool}")
                }));
            }
            let forwarded = json!({
                "name": tool,
                "arguments": arguments.get("arguments").cloned().unwrap_or_else(|| json!({}))
            });
            call_tool(&forwarded, workspace, jobs, permissions, runtime)
        }
        _ => Err(json!({"code": -32602, "message": format!("unknown tool: {name}")})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn protocol_lines_are_bounded_and_recoverable() {
        let oversized = "x".repeat(MAX_PROTOCOL_LINE_BYTES + 1);
        let mut input = Cursor::new(format!("{oversized}\n{{\"ok\":true}}\n"));
        let error = read_bounded_line(&mut input).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            read_bounded_line(&mut input).unwrap().as_deref(),
            Some("{\"ok\":true}")
        );
    }

    #[test]
    fn protocol_line_without_newline_is_accepted_at_the_limit() {
        let mut input = Cursor::new("x".repeat(MAX_PROTOCOL_LINE_BYTES));
        let line = read_bounded_line(&mut input).unwrap().unwrap();
        assert_eq!(line.len(), MAX_PROTOCOL_LINE_BYTES);
        assert!(read_bounded_line(&mut input).unwrap().is_none());
    }

    #[test]
    fn dependency_errors_are_machine_readable() {
        let payload = runtime_error(
            "search",
            RuntimeToolError::new(RuntimeErrorKind::Dependency, "ripgrep is missing"),
        );
        assert_eq!(payload["code"], -32012);
        assert_eq!(payload["reason_code"], "dependency_unavailable");
        assert_eq!(payload["failure_stage"], "dependency_resolution");
        assert_eq!(payload["retryable"], false);
    }

    #[test]
    fn dependency_errors_keep_per_tool_codes() {
        assert_eq!(
            runtime_error(
                "git",
                RuntimeToolError::new(RuntimeErrorKind::Dependency, "git is not available"),
            )["code"],
            -32042
        );
    }

    #[test]
    fn execution_errors_stay_distinct_from_dependency_errors() {
        let payload = runtime_error(
            "search",
            RuntimeToolError::new(RuntimeErrorKind::Execution, "search failed"),
        );
        assert_eq!(payload["code"], -32010);
        assert!(payload.get("reason_code").is_none());
        assert!(payload.get("retryable").is_none());
    }
}
