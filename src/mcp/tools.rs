use serde_json::{json, Value};

pub(super) fn list() -> Value {
    let mut response = json!({"tools": [
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
                    "queries": {"type": "array", "items": {"type": "string", "minLength": 1, "maxLength": 1024}, "minItems": 1, "maxItems": 8},
                    "max_results": {"type": "integer", "minimum": 1, "maximum": 200},
                    "literal": {"type": "boolean", "default": false},
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
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "additionalProperties": false}
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
            "inputSchema": {"type": "object", "properties": {"compact": {"type": "boolean"}, "client_id": {"type": "string"}}, "additionalProperties": true}
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
            "inputSchema": {"type": "object", "properties": {"tool_name": {"type": "string"}, "category": {"type": "string"}, "intent": {"type": "string"}, "include_recommended_flows": {"type": "boolean"}}, "additionalProperties": true}
        },
        {
            "name": "call_runtime_tool",
            "description": "Invoke one existing bounded web-harness runtime tool through the compatibility gateway.",
            "inputSchema": {
                "type": "object",
                "properties": {"tool": {"type": "string"}, "arguments": {"type": "object"}},
                "required": ["tool"],
                "additionalProperties": true
            }
        }
    ]});
    annotate(&mut response);
    response
}

fn annotate(response: &mut Value) {
    let Some(tools) = response.get_mut("tools").and_then(Value::as_array_mut) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_runtime_tool_has_a_direct_annotated_mcp_schema() {
        let listed = list();
        let names = listed["tools"].as_array().unwrap();
        for name in crate::runtime::registry::RuntimeRegistry::default().names() {
            let tool = names
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("runtime tool {name} is missing from tools/list"));
            assert!(tool["inputSchema"].is_object(), "{name} has no schema");
            assert!(tool["annotations"].is_object(), "{name} has no annotations");
        }
    }
}
