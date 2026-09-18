use crate::exec;
use crate::jobs::JobManager;
use crate::patch;
use crate::search;
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
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) if request.id.is_none() && request.method.starts_with("notifications/") => {
                continue;
            }
            Ok(request) => handle(request, &workspace, &mut jobs),
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

fn handle(request: Request, workspace: &Workspace, jobs: &mut JobManager) -> Response {
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
                        "max_results": {"type": "integer", "minimum": 1, "maximum": 200}
                    },
                    "required": ["query"],
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
                        "background": {"type": "boolean"}
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
            }
        ]})),
        "tools/call" => call_tool(&request.params, workspace, jobs),
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

fn call_tool(params: &Value, workspace: &Workspace, jobs: &mut JobManager) -> Result<Value, Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| json!({"code": -32602, "message": "missing tool name"}))?;
    match name {
        "workspace_info" => Ok(json!({
            "content": [{"type": "text", "text": serde_json::to_string(&workspace.info()).unwrap()}]
        })),
        "read_files" => {
            let paths = params
                .get("arguments")
                .and_then(|v| v.get("paths"))
                .and_then(Value::as_array)
                .ok_or_else(|| json!({"code": -32602, "message": "arguments.paths is required"}))?;
            let mut total = 0usize;
            let mut files = Vec::new();
            for path in paths {
                let path = path
                    .as_str()
                    .ok_or_else(|| json!({"code": -32602, "message": "path must be a string"}))?;
                let text = workspace
                    .read_text_bounded(path, 256 * 1024)
                    .map_err(|error| json!({"code": -32001, "message": error.to_string()}))?;
                total += text.len();
                if total > 512 * 1024 {
                    return Err(json!({"code": -32002, "message": "batch read exceeds 512 KiB"}));
                }
                files.push(json!({"path": path, "text": text}));
            }
            Ok(
                json!({"content": [{"type": "text", "text": serde_json::to_string(&json!({"files": files})).unwrap()}]}),
            )
        }
        "search" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| json!({"code": -32602, "message": "arguments.query is required"}))?;
            let max_results = arguments
                .get("max_results")
                .and_then(Value::as_u64)
                .unwrap_or(100) as usize;
            let matches = search::content_search(workspace, query, max_results)
                .map_err(|error| json!({"code": -32010, "message": error.to_string()}))?;
            Ok(
                json!({"content": [{"type": "text", "text": serde_json::to_string(&json!({"matches": matches})).unwrap()}]}),
            )
        }
        "workspace_instructions" => {
            let path = params
                .get("arguments")
                .and_then(|value| value.get("path"))
                .and_then(Value::as_str)
                .unwrap_or(".");
            let files = workspace
                .discover_agents(path)
                .map_err(|error| json!({"code": -32011, "message": error.to_string()}))?;
            let mut instructions = Vec::new();
            for file in files {
                let relative = file
                    .strip_prefix(workspace.root())
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                let text = workspace
                    .read_text_bounded(&relative, 128 * 1024)
                    .map_err(|error| json!({"code": -32011, "message": error.to_string()}))?;
                instructions.push(json!({"path": relative, "text": text}));
            }
            Ok(
                json!({"content": [{"type": "text", "text": serde_json::to_string(&json!({"instructions": instructions})).unwrap()}]}),
            )
        }
        "patch" => {
            let patch_text = params
                .get("arguments")
                .and_then(|value| value.get("patch"))
                .and_then(Value::as_str)
                .ok_or_else(|| json!({"code": -32602, "message": "arguments.patch is required"}))?;
            if patch_text.len() > 512 * 1024 {
                return Err(json!({"code": -32020, "message": "patch exceeds 512 KiB"}));
            }
            let changed_paths = patch::apply(workspace, patch_text)
                .map_err(|error| json!({"code": -32021, "message": error.to_string()}))?;
            Ok(
                json!({"content": [{"type": "text", "text": serde_json::to_string(&json!({"changed_paths": changed_paths})).unwrap()}]}),
            )
        }
        "exec" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let argv = arguments
                .get("argv")
                .and_then(Value::as_array)
                .ok_or_else(|| json!({"code": -32602, "message": "arguments.argv is required"}))?
                .iter()
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(
                        || json!({"code": -32602, "message": "argv items must be strings"}),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let cwd = arguments.get("cwd").and_then(Value::as_str);
            let background = arguments
                .get("background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let value = if background {
                serde_json::to_value(
                    exec::background(jobs, workspace, &argv, cwd)
                        .map_err(|error| json!({"code": -32030, "message": error.to_string()}))?,
                )
            } else {
                serde_json::to_value(
                    exec::foreground(
                        jobs,
                        workspace,
                        &argv,
                        cwd,
                        arguments.get("timeout_ms").and_then(Value::as_u64),
                    )
                    .map_err(|error| json!({"code": -32030, "message": error.to_string()}))?,
                )
            }
            .map_err(|error| json!({"code": -32603, "message": error.to_string()}))?;
            Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
        }
        "job" => {
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let action = arguments.get("action").and_then(Value::as_str).ok_or_else(
                || json!({"code": -32602, "message": "arguments.action is required"}),
            )?;
            let id = arguments
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| json!({"code": -32602, "message": "arguments.id is required"}))?;
            let value = match action {
                "poll" => serde_json::to_value(
                    jobs.poll(id)
                        .map_err(|error| json!({"code": -32031, "message": error.to_string()}))?,
                )
                .map_err(|error| json!({"code": -32603, "message": error.to_string()}))?,
                "cancel" => serde_json::to_value(
                    jobs.cancel(id)
                        .map_err(|error| json!({"code": -32031, "message": error.to_string()}))?,
                )
                .map_err(|error| json!({"code": -32603, "message": error.to_string()}))?,
                "output" => {
                    let stream = arguments
                        .get("stream")
                        .and_then(Value::as_str)
                        .unwrap_or("stdout");
                    let (text, truncated) = jobs
                        .output(id, stream)
                        .map_err(|error| json!({"code": -32031, "message": error.to_string()}))?;
                    json!({"stream": stream, "text": text, "truncated": truncated})
                }
                _ => {
                    return Err(json!({"code": -32602, "message": "unknown job action"}));
                }
            };
            Ok(json!({"content": [{"type": "text", "text": value.to_string()}]}))
        }
        _ => Err(json!({"code": -32602, "message": format!("unknown tool: {name}")})),
    }
}
