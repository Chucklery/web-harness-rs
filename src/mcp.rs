use crate::jobs::JobManager;
use crate::permission::PermissionEngine;
use crate::runtime::{
    manifest, registry::RuntimeRegistry, status, ExecutionContext, RuntimeErrorKind,
    RuntimeToolError,
};
use crate::workspace::Workspace;
use approval::{HostDecision, Session};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

mod approval;
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

#[derive(Debug, Serialize)]
pub(super) struct Response {
    jsonrpc: &'static str,
    pub(super) id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

pub fn serve_stdio(workspace: Workspace) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout().lock();
    serve(&workspace, &mut stdin, &mut stdout)
}

fn serve<R: BufRead, W: Write>(
    workspace: &Workspace,
    stdin: &mut R,
    stdout: &mut W,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut jobs = JobManager::new();
    let mut permissions = PermissionEngine::for_workspace(workspace)?;
    let runtime = RuntimeRegistry::default();
    let mut session = Session::new();
    loop {
        let line = match read_bounded_line(stdin) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                let response = Response {
                    jsonrpc: "2.0",
                    id: None,
                    result: None,
                    error: Some(json!({"code": -32700, "message": error.to_string()})),
                };
                serde_json::to_writer(&mut *stdout, &response)?;
                writeln!(stdout)?;
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
                serde_json::to_writer(&mut *stdout, &response)?;
                writeln!(stdout)?;
                stdout.flush()?;
                continue;
            }
        };
        if request.method == "initialize" {
            session.note_initialize(&request.params);
        }
        let mut response = handle(
            request.clone(),
            workspace,
            &mut jobs,
            &mut permissions,
            &runtime,
        );
        let mut retry = request.clone();
        let mut suppress_response = false;
        while session.supports_elicitation() && request.method == "tools/call" {
            let Some(pending) = approval::pending_approval(&response) else {
                break;
            };
            match approval::request_host_approval(
                stdin,
                stdout,
                &mut session,
                &response,
                request.id.as_ref(),
            )? {
                HostDecision::Accept => {
                    permissions.approve(&pending.id)?;
                    let arguments = retry
                        .params
                        .get_mut("arguments")
                        .and_then(Value::as_object_mut)
                        .ok_or("tools/call arguments must be an object")?;
                    arguments.insert(pending.argument, Value::String(pending.id));
                    response = handle(
                        retry.clone(),
                        workspace,
                        &mut jobs,
                        &mut permissions,
                        &runtime,
                    );
                }
                HostDecision::Decline => {
                    revoke_request_approvals(&mut permissions, &retry, &pending.id);
                    response = denied_response(&response);
                    break;
                }
                HostDecision::Cancel => {
                    revoke_request_approvals(&mut permissions, &retry, &pending.id);
                    suppress_response = true;
                    break;
                }
                HostDecision::Disconnected => break,
            }
        }
        if suppress_response {
            continue;
        }
        serde_json::to_writer(&mut *stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

fn revoke_request_approvals(
    permissions: &mut PermissionEngine,
    request: &Request,
    current_ticket: &str,
) {
    let mut ticket_ids = vec![current_ticket.to_string()];
    if let Some(arguments) = request.params.get("arguments").and_then(Value::as_object) {
        for argument in ["approval_id", "git_approval_id", "network_approval_id"] {
            if let Some(ticket_id) = arguments.get(argument).and_then(Value::as_str) {
                ticket_ids.push(ticket_id.to_string());
            }
        }
    }
    for ticket_id in ticket_ids {
        let _ = permissions.deny(&ticket_id);
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
    let mut result = runtime_content(json!({
        "status": "denied",
        "capability": capability
    }));
    result["isError"] = json!(true);
    Response {
        jsonrpc: "2.0",
        id: response.id.clone(),
        result: Some(result),
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
                let requested = match std::fs::canonicalize(path) {
                    Ok(requested) => requested,
                    Err(error) => {
                        let kind = if error.kind() == io::ErrorKind::NotFound {
                            RuntimeErrorKind::NotFound
                        } else {
                            RuntimeErrorKind::Workspace
                        };
                        return Ok(runtime_tool_error(
                            name,
                            RuntimeToolError::new(
                                kind,
                                "requested workspace path could not be resolved",
                            ),
                        ));
                    }
                };
                if requested != workspace.root() {
                    return Ok(runtime_tool_error(
                        name,
                        RuntimeToolError::new(
                            RuntimeErrorKind::Denied,
                            "web-harness is bound to one configured workspace; requested path does not match it",
                        ),
                    ));
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
            Ok(match manifest::describe(runtime, requested) {
                Ok(value) => runtime_content(value),
                Err(error) => runtime_tool_error("tool_manifest", error),
            })
        }
        "call_runtime_tool" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let Some(tool) = arguments.get("tool").and_then(Value::as_str) else {
                return Ok(runtime_tool_error(
                    name,
                    RuntimeToolError::new(
                        RuntimeErrorKind::InvalidArguments,
                        "arguments.tool is required",
                    ),
                ));
            };
            if !runtime.contains(tool) {
                return Ok(runtime_tool_error(
                    name,
                    RuntimeToolError::new(
                        RuntimeErrorKind::InvalidArguments,
                        format!(
                            "runtime tool is not exposed through the compatibility gateway: {tool}"
                        ),
                    ),
                ));
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

    #[test]
    fn cancellation_revokes_all_tickets_collected_for_the_request() {
        let mut permissions = PermissionEngine::new().unwrap();
        let mut ticket_ids = Vec::new();
        for capability in [
            crate::permission::Capability::ProcessExecute,
            crate::permission::Capability::GitLocalWrite,
            crate::permission::Capability::NetworkOutbound,
        ] {
            let ticket = permissions
                .request_action(
                    &crate::permission::ExecAuthorization {
                        capability,
                        argv: vec!["sh".into()],
                        cwd: None,
                        background: false,
                        network: crate::sandbox::NetworkPolicy::Deny,
                        expected_head: None,
                        stdin: Some("git add file".into()),
                        protected_read: false,
                    },
                    "test approval".into(),
                    "test".into(),
                )
                .unwrap();
            ticket_ids.push(ticket.id);
        }
        let request = Request {
            jsonrpc: "2.0".into(),
            id: Some(json!(12)),
            method: "tools/call".into(),
            params: json!({"arguments": {
                "approval_id": ticket_ids[0],
                "git_approval_id": ticket_ids[1]
            }}),
        };

        revoke_request_approvals(&mut permissions, &request, &ticket_ids[2]);

        for ticket_id in ticket_ids {
            assert!(permissions.deny(&ticket_id).is_err());
        }
    }

    #[test]
    fn cancelled_approval_wait_suppresses_call_response_and_resumes_stdio() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let requests = [
            json!({
                "jsonrpc":"2.0", "id":1, "method":"initialize",
                "params":{"capabilities":{"elicitation":{}}}
            }),
            json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{"name":"exec", "arguments":{"script":"printf cancelled"}}
            }),
            json!({
                "jsonrpc":"2.0", "method":"notifications/cancelled",
                "params":{"requestId":2, "reason":"user cancelled"}
            }),
            json!({"jsonrpc":"2.0", "id":3, "method":"tools/list"}),
        ];
        let input = requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut input = Cursor::new(format!("{input}\n"));
        let mut output = Vec::new();

        serve(&workspace, &mut input, &mut output).unwrap();

        let responses = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0]["id"], 1);
        assert_eq!(responses[1]["method"], "elicitation/create");
        assert_eq!(responses[2]["id"], 3);
        assert!(responses.iter().all(|response| response["id"] != 2));
    }

    #[test]
    fn declined_approval_returns_a_denied_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let requests = [
            json!({
                "jsonrpc":"2.0", "id":1, "method":"initialize",
                "params":{"capabilities":{"elicitation":{}}}
            }),
            json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{"name":"exec", "arguments":{"script":"printf declined"}}
            }),
            json!({
                "jsonrpc":"2.0", "id":"web_harness_elicitation_1",
                "result":{"action":"decline"}
            }),
        ];
        let input = requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut input = Cursor::new(format!("{input}\n"));
        let mut output = Vec::new();

        serve(&workspace, &mut input, &mut output).unwrap();

        let responses = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 3);
        assert_eq!(responses[2]["id"], 2);
        assert_eq!(
            responses[2]["result"]["structuredContent"]["status"],
            "denied"
        );
        assert_eq!(responses[2]["result"]["isError"], true);
    }
}
