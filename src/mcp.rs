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

#[derive(Debug, Deserialize)]
struct Request {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
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
    let mut stdout = io::stdout().lock();
    let mut jobs = JobManager::new();
    let mut permissions = PermissionEngine::new()?;
    let runtime = RuntimeRegistry::default();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) if request.id.is_none() && request.method.starts_with("notifications/") => {
                continue;
            }
            Ok(request) => handle(request, &workspace, &mut jobs, &mut permissions, &runtime),
            Err(error) => Response {
                jsonrpc: "2.0",
                id: None,
                result: None,
                error: Some(json!({"code": -32700, "message": error.to_string()})),
            },
        };
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(&mut stdout)?;
        stdout.flush()?;
    }
    Ok(())
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
        "tools/list" => Ok(json!({"tools": [
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
                "description": "Apply a bounded Codex-style structured patch inside the workspace.",
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
                        "approval_id": {"type": "string"}
                    },
                    "oneOf": [{"required": ["argv"]}, {"required": ["script"]}],
                    "additionalProperties": false
                }
            },
            {
                "name": "job",
                "description": "Poll, read output, or cancel a background job.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["poll", "wait", "output", "cancel", "list"]},
                        "id": {"type": "string"},
                        "stream": {"type": "string", "enum": ["stdout", "stderr"]},
                        "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 60000},
                        "cursor": {"type": "integer", "minimum": 0}
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
                "name": "permission",
                "description": "Approve or deny a one-time execution approval ticket. Approval must be confirmed by the user through the MCP host.",
                "annotations": {
                    "readOnlyHint": false,
                    "destructiveHint": true,
                    "openWorldHint": false
                },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["approve", "deny"]},
                        "id": {"type": "string"}
                    },
                    "required": ["action", "id"],
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
        ]})),
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
        return result
            .map(runtime_content)
            .map_err(|error| runtime_error(name, error));
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
            if tool == "permission" {
                return Err(json!({
                    "code": -32602,
                    "message": "permission is direct-only so MCP host confirmation cannot be bypassed"
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
