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
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) if request.id.is_none() && request.method.starts_with("notifications/") => {
                continue;
            }
            Ok(request) => handle(request, &workspace),
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

fn handle(request: Request, workspace: &Workspace) -> Response {
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
            }
        ]})),
        "tools/call" => call_tool(&request.params, workspace),
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

fn call_tool(params: &Value, workspace: &Workspace) -> Result<Value, Value> {
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
        _ => Err(json!({"code": -32602, "message": format!("unknown tool: {name}")})),
    }
}
