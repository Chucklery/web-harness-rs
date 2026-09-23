use super::{read_bounded_line, Response};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

#[derive(Default)]
pub(super) struct Session {
    next_server_request_id: u64,
    elicitation_supported: bool,
}

impl Session {
    pub(super) fn new() -> Self {
        Self {
            next_server_request_id: 1,
            elicitation_supported: false,
        }
    }

    pub(super) fn note_initialize(&mut self, params: &Value) {
        self.elicitation_supported = params
            .get("capabilities")
            .and_then(|capabilities| capabilities.get("elicitation"))
            .is_some_and(Value::is_object);
    }

    pub(super) fn supports_elicitation(&self) -> bool {
        self.elicitation_supported
    }
}

pub(super) struct PendingApproval {
    pub(super) id: String,
    pub(super) argument: String,
}

pub(super) fn pending_approval(response: &Response) -> Option<PendingApproval> {
    let structured = response.result.as_ref()?.get("structuredContent")?;
    if structured.get("status")?.as_str()? != "approval_required" {
        return None;
    }
    Some(PendingApproval {
        id: structured.get("approval")?.get("id")?.as_str()?.to_string(),
        argument: structured
            .get("approval_argument")
            .and_then(Value::as_str)
            .unwrap_or("approval_id")
            .to_string(),
    })
}

pub(super) enum HostDecision {
    Accept,
    Decline,
    Cancel,
    Disconnected,
}

pub(super) fn request_host_approval<R: BufRead, W: Write>(
    stdin: &mut R,
    stdout: &mut W,
    session: &mut Session,
    response: &Response,
    parent_request_id: Option<&Value>,
) -> io::Result<HostDecision> {
    let structured = response
        .result
        .as_ref()
        .and_then(|result| result.get("structuredContent"))
        .ok_or_else(|| io::Error::other("approval response is missing structured content"))?;
    let approval = structured.get("approval");
    let summary = approval
        .and_then(|approval| approval.get("summary"))
        .and_then(Value::as_str)
        .unwrap_or("Confirm this protected operation");
    let capability = structured
        .get("capability")
        .and_then(Value::as_str)
        .unwrap_or("protected operation");
    let reason = approval
        .and_then(|approval| approval.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("This operation requires user confirmation");
    let message = format!(
        "Capability: {capability}. {summary}. {reason}. Review the original tool request before approving; this approval is one-time."
    );
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
            Ok(None) => return Ok(HostDecision::Disconnected),
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
            && parent_request_id.is_some_and(|request_id| {
                value
                    .get("params")
                    .and_then(|params| params.get("requestId"))
                    == Some(request_id)
            })
        {
            return Ok(HostDecision::Cancel);
        }
        if value.get("id") != Some(&json!(request_id)) {
            continue;
        }
        let Some(result) = value.get("result") else {
            return Ok(HostDecision::Decline);
        };
        return match result.get("action").and_then(Value::as_str) {
            Some("accept")
                if result
                    .get("content")
                    .and_then(|content| content.get("approved"))
                    .and_then(Value::as_bool)
                    == Some(true) =>
            {
                Ok(HostDecision::Accept)
            }
            Some("cancel") => Ok(HostDecision::Cancel),
            _ => Ok(HostDecision::Decline),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn pending_response() -> Response {
        Response {
            jsonrpc: "2.0",
            id: Some(json!(17)),
            result: Some(json!({
                "structuredContent": {
                    "status": "approval_required",
                    "capability": "process.execute",
                    "approval": {
                        "id": "ticket-1",
                        "summary": "Run test; inspect arguments",
                        "reason": "This command can execute code"
                    },
                    "approval_argument": "approval_id"
                }
            })),
            error: None,
        }
    }

    fn run_decision(message: &str) -> HostDecision {
        let mut input = Cursor::new(message.to_string());
        let mut output = Vec::new();
        let mut session = Session::new();
        request_host_approval(
            &mut input,
            &mut output,
            &mut session,
            &pending_response(),
            Some(&json!(17)),
        )
        .unwrap()
    }

    #[test]
    fn parent_cancellation_is_distinguished_from_unrelated_notifications() {
        let decision = run_decision(concat!(
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":99}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":17}}\n"
        ));
        assert!(matches!(decision, HostDecision::Cancel));
    }

    #[test]
    fn elicitation_decline_and_cancel_remain_distinct() {
        assert!(matches!(
            run_decision(
                "{\"id\":\"web_harness_elicitation_1\",\"result\":{\"action\":\"decline\"}}\n"
            ),
            HostDecision::Decline
        ));
        assert!(matches!(
            run_decision(
                "{\"id\":\"web_harness_elicitation_1\",\"result\":{\"action\":\"cancel\"}}\n"
            ),
            HostDecision::Cancel
        ));
    }

    #[test]
    fn elicitation_accept_requires_explicit_true_value() {
        assert!(matches!(
            run_decision("{\"id\":\"web_harness_elicitation_1\",\"result\":{\"action\":\"accept\",\"content\":{\"approved\":true}}}\n"),
            HostDecision::Accept
        ));
        assert!(matches!(
            run_decision("{\"id\":\"web_harness_elicitation_1\",\"result\":{\"action\":\"accept\",\"content\":{\"approved\":false}}}\n"),
            HostDecision::Decline
        ));
    }

    #[test]
    fn elicitation_shows_capability_reason_and_review_reminder() {
        let mut input = Cursor::new(
            "{\"id\":\"web_harness_elicitation_1\",\"result\":{\"action\":\"decline\"}}\n"
                .to_string(),
        );
        let mut output = Vec::new();
        let mut session = Session::new();
        request_host_approval(
            &mut input,
            &mut output,
            &mut session,
            &pending_response(),
            Some(&json!(17)),
        )
        .unwrap();
        let request: Value = serde_json::from_slice(&output).unwrap();
        let message = request["params"]["message"].as_str().unwrap();
        assert!(message.contains("process.execute"));
        assert!(message.contains("This command can execute code"));
        assert!(message.contains("Review the original tool request"));
    }

    #[test]
    fn disconnect_during_elicitation_does_not_approve() {
        let decision = run_decision("");
        assert!(matches!(decision, HostDecision::Disconnected));
    }
}
