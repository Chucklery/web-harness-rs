use super::approval::{self, HostDecision, Session};
use super::{denied_response, handle, read_bounded_line, Request, Response};
use crate::jobs::JobManager;
use crate::permission::PermissionEngine;
use crate::runtime::registry::RuntimeRegistry;
use crate::workspace::Workspace;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub(super) fn serve<R: BufRead, W: Write>(
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
                write_response(stdout, &response)?;
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
                write_response(stdout, &response)?;
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
        if !suppress_response {
            write_response(stdout, &response)?;
        }
    }
    Ok(())
}

fn write_response<W: Write>(stdout: &mut W, response: &Response) -> io::Result<()> {
    serde_json::to_writer(&mut *stdout, response)?;
    writeln!(stdout)?;
    stdout.flush()
}

pub(super) fn revoke_request_approvals(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::MAX_PROTOCOL_LINE_BYTES;
    use crate::permission::{Capability, ExecAuthorization};
    use crate::sandbox::NetworkPolicy;
    use std::io::Cursor;

    fn test_script(output: &str) -> Value {
        json!({
            "script": if cfg!(windows) {
                format!("Write-Output '{output}'")
            } else {
                format!("printf '{output}\\n'")
            },
            "shell": if cfg!(windows) { "pwsh" } else { "sh" }
        })
    }

    #[test]
    fn write_response_emits_one_flushed_jsonrpc_line() {
        let response = Response {
            jsonrpc: "2.0",
            id: Some(json!(1)),
            result: Some(json!({"ok": true})),
            error: None,
        };
        let mut output = Vec::new();
        write_response(&mut output, &response).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap(),
            json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}})
        );
        assert_eq!(output.last(), Some(&b'\n'));
    }

    #[test]
    fn protocol_line_limit_remains_owned_by_transport_boundary() {
        assert_eq!(MAX_PROTOCOL_LINE_BYTES, 2 * 1024 * 1024);
    }

    #[test]
    fn cancellation_revokes_all_tickets_collected_for_the_request() {
        let mut permissions = PermissionEngine::new().unwrap();
        let mut ticket_ids = Vec::new();
        for capability in [
            Capability::ProcessExecute,
            Capability::GitLocalWrite,
            Capability::NetworkOutbound,
        ] {
            let ticket = permissions
                .request_action(
                    &ExecAuthorization {
                        capability,
                        argv: vec!["sh".into()],
                        cwd: None,
                        background: false,
                        network: NetworkPolicy::Deny,
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
                "params":{"name":"exec", "arguments":test_script("cancelled")}
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
                "params":{"name":"exec", "arguments":test_script("declined")}
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
