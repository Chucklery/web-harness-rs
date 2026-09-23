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

mod tools;

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
        "tools/list" => Ok(tools::list()),
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
