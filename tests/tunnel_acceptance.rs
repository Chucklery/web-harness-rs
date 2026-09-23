use serde_json::Value;
use std::process::Command;

#[test]
fn successful_external_exit_without_evidence_is_not_acceptance() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let workspace = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let command_json = r#"["cmd","/C","exit 0"]"#;
    #[cfg(not(windows))]
    let command_json = r#"["/usr/bin/true"]"#;

    let output = Command::new(binary)
        .args([
            "tunnel",
            "accept",
            "--workspace",
            workspace.path().to_str().unwrap(),
            "--command-json",
            command_json,
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["local_mcp_roundtrip"], true);
    assert_eq!(report["external_exit_code"], 0);
    assert_eq!(report["external_evidence_valid"], false);
    assert_eq!(report["external_command_passed"], false);
    assert_eq!(report["passed"], false);
    assert!(report["external_evidence"].is_null());
}

#[cfg(unix)]
#[test]
fn reports_complete_bounded_external_evidence() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let workspace = tempfile::tempdir().unwrap();
    let stages = [
        "connect_workspace",
        "discover_project",
        "read_instructions_and_source",
        "search_symbol",
        "modify_workspace",
        "run_validation",
        "observe_background_job",
        "inspect_git_changes",
        "verify_user_approval",
        "disconnect_cleanup",
    ]
    .map(|stage| serde_json::json!({"stage":stage,"passed":true}));
    let tools = [
        ("workspace_info", "succeeded"),
        ("list_files", "succeeded"),
        ("workspace_instructions", "succeeded"),
        ("read_files", "succeeded"),
        ("search", "succeeded"),
        ("patch", "succeeded"),
        ("exec", "approval_required"),
        ("exec", "succeeded"),
        ("job_wait", "succeeded"),
        ("git_status", "succeeded"),
        ("git_diff", "succeeded"),
    ];
    let tool_calls = tools
        .iter()
        .enumerate()
        .map(|(index, (tool, outcome))| {
            serde_json::json!({
                "sequence": index + 1,
                "tool": tool,
                "outcome": outcome,
                "error_code": null
            })
        })
        .collect::<Vec<_>>();
    let evidence = serde_json::json!({
        "schema_version": 1,
        "client_version": "1.2026.09.23",
        "protocol_version": "2025-06-18",
        "stages": stages,
        "tool_calls": tool_calls,
        "failure_recovery": "not_needed"
    })
    .to_string();
    let command_json = serde_json::json!(["/usr/bin/printf", "%s", evidence]).to_string();

    let output = Command::new(binary)
        .args([
            "tunnel",
            "accept",
            "--workspace",
            workspace.path().to_str().unwrap(),
            "--command-json",
            &command_json,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], true);
    assert_eq!(report["external_evidence_valid"], true);
    assert_eq!(
        report["external_evidence"]["client_version"],
        "1.2026.09.23"
    );
    assert_eq!(
        report["external_evidence"]["tool_calls"]
            .as_array()
            .unwrap()
            .len(),
        11
    );
}
