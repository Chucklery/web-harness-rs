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
        ("workspace_info", None, "succeeded"),
        ("list_files", None, "succeeded"),
        ("workspace_instructions", None, "succeeded"),
        ("read_files", None, "succeeded"),
        ("search", None, "succeeded"),
        ("patch", None, "succeeded"),
        ("exec", None, "approval_required"),
        ("exec", None, "succeeded"),
        ("job", Some("wait"), "succeeded"),
        ("git", Some("status"), "succeeded"),
        ("git", Some("diff"), "succeeded"),
    ];
    let tool_calls = tools
        .iter()
        .enumerate()
        .map(|(index, (tool, action, outcome))| {
            serde_json::json!({
                "sequence": index + 1,
                "tool": tool,
                "action": action,
                "outcome": outcome,
                "error_code": null
            })
        })
        .collect::<Vec<_>>();
    let evidence = serde_json::json!({
        "schema_version": 2,
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

#[cfg(unix)]
#[test]
fn tunnel_accept_terminates_wrapper_descendants_before_draining_output() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let binary = env!("CARGO_BIN_EXE_web-harness");
    let workspace = tempfile::tempdir().unwrap();
    let wrapper = workspace.path().join("wrapper.sh");
    std::fs::write(
        &wrapper,
        "#!/bin/sh\necho $$ > \"$WEB_HARNESS_WORKSPACE/wrapper.pid\"\n/bin/sleep 60 &\necho $! > \"$WEB_HARNESS_WORKSPACE/sleeper.pid\"\nprintf '{}'\n",
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let command_json = serde_json::json!([wrapper.display().to_string()]).to_string();
    let mut child = Command::new(binary)
        .args([
            "tunnel",
            "accept",
            "--workspace",
            workspace.path().to_str().unwrap(),
            "--command-json",
            &command_json,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            terminate_wrapper_group(workspace.path());
            let _ = child.kill();
            let _ = child.wait();
            panic!("tunnel accept hung while a wrapper descendant held stdout open");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    assert_eq!(status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["external_command_passed"], false);

    let sleeper_pid = std::fs::read_to_string(workspace.path().join("sleeper.pid"))
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    while pid_is_alive(sleeper_pid) && Instant::now() < reap_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if pid_is_alive(sleeper_pid) {
        terminate_wrapper_group(workspace.path());
    }
    assert!(!pid_is_alive(sleeper_pid));
}

#[cfg(unix)]
fn terminate_wrapper_group(workspace: &std::path::Path) {
    if let Ok(pid) = std::fs::read_to_string(workspace.join("wrapper.pid")) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
fn pid_is_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}
