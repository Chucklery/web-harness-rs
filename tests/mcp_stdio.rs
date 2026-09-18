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
