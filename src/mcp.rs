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
                    "properties": {"paths": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 16}},
                    "required": ["paths"],
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
                    "properties": {"patch": {"type": "string", "maxLength": 524288}},
                    "required": ["patch"],
                    "additionalProperties": false
                }
            },
            {
                "name": "exec",
                "description": "Execute a bounded argv command in the workspace, foreground or background.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 64},
                        "cwd": {"type": "string"},
                        "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 600000},
                        "background": {"type": "boolean"},
                        "approval_id": {"type": "string"}
                    },
                    "required": ["argv"],
                    "additionalProperties": false
                }
            },
            {
                "name": "job",
                "description": "Poll, read output, or cancel a background job.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["poll", "output", "cancel"]},
                        "id": {"type": "string"},
                        "stream": {"type": "string", "enum": ["stdout", "stderr"]}
                    },
                    "required": ["action", "id"],
                    "additionalProperties": false
                }
            },
            {
                "name": "git",
                "description": "Run structured Git operations. Mutating actions require one-time approval.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["status", "diff", "log", "show", "add", "commit", "switch", "restore", "push"]},
                        "staged": {"type": "boolean"},
                        "pathspec": {"type": "array", "items": {"type": "string"}, "maxItems": 32},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                        "revision": {"type": "string", "maxLength": 256},
                        "message": {"type": "string", "minLength": 1, "maxLength": 4096},
                        "branch": {"type": "string", "minLength": 1, "maxLength": 256},
                        "remote": {"type": "string", "minLength": 1, "maxLength": 256},
                        "refspec": {"type": "string", "minLength": 1, "maxLength": 256},
                        "approval_id": {"type": "string"}
                    },
                    "required": ["action"],
                    "additionalProperties": false
                }
            },
            {
                "name": "permission",
                "description": "Approve or deny a one-time execution approval ticket.",
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
    json!({"content": [{"type": "text", "text": value.to_string()}]})
}

fn runtime_error(tool: &str, error: RuntimeToolError) -> Value {
    let code = match error.kind() {
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
    };
    json!({"code": code, "message": error.message()})
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
            Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
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
