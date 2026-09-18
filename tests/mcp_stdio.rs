use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn stdio_mcp_initializes_and_lists_core_tools() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let mut child = Command::new(binary)
        .args(["serve", "--stdio", "--workspace", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )
    .unwrap();
    stdin.flush().unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let initialize: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(initialize["result"]["serverInfo"]["name"], "web-harness");

    let tools: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for expected in [
        "workspace_info",
        "read_files",
        "search",
        "workspace_instructions",
        "patch",
        "exec",
        "job",
        "git",
        "permission",
    ] {
        assert!(names.contains(&expected), "missing MCP tool: {expected}");
    }

    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn git_mutation_requires_and_consumes_approval() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let dir = tempfile::tempdir().unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success());
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

    let mut child = Command::new(binary)
        .args([
            "serve",
            "--stdio",
            "--workspace",
            dir.path().to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();

    writeln!(
        stdin,
        "{}",
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
    )
    .unwrap();
    stdin.flush().unwrap();
    let _: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"git","arguments":{"action":"add","pathspec":["a.txt"]}}
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let approval_response: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let approval_text = approval_response["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let approval: Value = serde_json::from_str(approval_text).unwrap();
    assert_eq!(approval["status"], "approval_required");
    let approval_id = approval["approval"]["id"].as_str().unwrap().to_string();

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"permission","arguments":{"action":"approve","id":approval_id}}
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let _: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params":{
                "name":"git",
                "arguments":{"action":"add","pathspec":["a.txt"],"approval_id":approval_id}
            }
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let result: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert!(result.get("error").is_none());

    drop(stdin);
    assert!(child.wait().unwrap().success());
    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&staged.stdout).trim(), "a.txt");
}
