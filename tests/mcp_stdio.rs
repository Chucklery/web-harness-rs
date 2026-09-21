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
        "list_files",
        "search",
        "workspace_instructions",
        "patch",
        "exec",
        "job",
        "git",
        "runtime_status",
        "work_on_project",
        "tool_manifest",
        "call_runtime_tool",
    ] {
        assert!(names.contains(&expected), "missing MCP tool: {expected}");
    }
    assert!(!names.contains(&"permission"));

    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn adaptive_runtime_control_tools_are_callable() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hello shim").unwrap();

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

    for request in [
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"runtime_status","arguments":{"compact":true}}
        }),
        serde_json::json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"work_on_project","arguments":{"path":dir.path(),"instruction":"inspect"}}
        }),
        serde_json::json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"tool_manifest","arguments":{"tool_name":"read_files"}}
        }),
        serde_json::json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{
                "name":"call_runtime_tool",
                "arguments":{"tool":"read_files","arguments":{"paths":["hello.txt"]}}
            }
        }),
        serde_json::json!({
            "jsonrpc":"2.0","id":5,"method":"tools/call",
            "params":{"name":"read_files","arguments":{"paths":["hello.txt"]}}
        }),
        serde_json::json!({
            "jsonrpc":"2.0","id":6,"method":"tools/call",
            "params":{
                "name":"call_runtime_tool",
                "arguments":{"tool":"permission","arguments":{"action":"approve","id":"apr_x"}}
            }
        }),
    ] {
        writeln!(stdin, "{}", request).unwrap();
    }
    stdin.flush().unwrap();

    let status: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let status_text = status["result"]["content"][0]["text"].as_str().unwrap();
    assert!(status_text.contains(r#""runtime_exposure":"adaptive_shim""#));

    let project: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert!(project.get("error").is_none());

    let manifest: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let manifest_text = manifest["result"]["content"][0]["text"].as_str().unwrap();
    assert!(manifest_text.contains(r#""name":"read_files""#));

    let routed: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let routed_text = routed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(routed_text.contains("hello shim"));

    let direct: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let direct_text = direct["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(direct_text, routed_text);
    assert_eq!(
        direct["result"]["structuredContent"]["files"][0]["text"],
        "hello shim"
    );

    let permission_bypass: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(permission_bypass["error"]["code"], -32602);
    assert!(permission_bypass["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not exposed"));

    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn stdio_mcp_batches_search_queries_with_one_result_budget() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let dir = tempfile::tempdir().unwrap();
    let tool_dir = tempfile::tempdir().unwrap();
    let rg_path = tool_dir.path().join("rg");
    std::fs::write(
        &rg_path,
        r#"#!/bin/sh
case " $* " in
  *" needle-alpha "*)
    printf '%s\n' '{"type":"match","data":{"path":{"text":"./alpha.txt"},"lines":{"text":"needle-alpha\n"},"line_number":1}}'
    ;;
  *" needle-beta "*)
    printf '%s\n' '{"type":"match","data":{"path":{"text":"./beta.txt"},"lines":{"text":"needle-beta\n"},"line_number":1}}'
    ;;
esac
"#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&rg_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&rg_path, permissions).unwrap();
    }
    std::fs::write(
        dir.path().join("alpha.txt"),
        "needle-alpha\nneedle-shared\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("beta.txt"), "needle-beta\nneedle-shared\n").unwrap();

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
        .env(
            "PATH",
            format!(
                "{}:{}",
                tool_dir.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{
                "name":"search",
                "arguments":{
                    "queries":["needle-alpha","needle-beta"],
                    "max_results":10
                }
            }
        })
    )
    .unwrap();
    stdin.flush().unwrap();

    let response: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let payload: Value = serde_json::from_str(text).unwrap();
    let results = payload["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["query"], "needle-alpha");
    assert_eq!(results[1]["query"], "needle-beta");
    assert_eq!(results[0]["matches"][0]["path"], "./alpha.txt");
    assert_eq!(results[1]["matches"][0]["path"], "./beta.txt");
    let total_matches: usize = results
        .iter()
        .map(|result| result["matches"].as_array().unwrap().len())
        .sum();
    assert!(total_matches <= 10);

    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn stdio_mcp_rejects_search_query_and_queries_together() {
    let binary = env!("CARGO_BIN_EXE_web-harness");
    let mut child = Command::new(binary)
        .args(["serve", "--stdio", "--workspace", "."])
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
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{
                "name":"search",
                "arguments":{
                    "query":"alpha",
                    "queries":["beta"],
                    "max_results":10
                }
            }
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let response: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        -32602
    );

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
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"capabilities":{"elicitation":{"form":{}}}}
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
            "id":2,
            "method":"tools/call",
            "params":{"name":"git","arguments":{"action":"add","pathspec":["a.txt"]}}
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let elicitation: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(elicitation["method"], "elicitation/create");
    let elicitation_id = elicitation["id"].clone();

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":elicitation_id,
            "result":{"action":"accept","content":{"approved":true}}
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let result: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(result["id"], 2);
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

#[test]
fn host_decline_does_not_run_git_mutation() {
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
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"capabilities":{"elicitation":{"form":{}}}}
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
            "id":2,
            "method":"tools/call",
            "params":{"name":"git","arguments":{"action":"add","pathspec":["a.txt"]}}
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let elicitation: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(elicitation["method"], "elicitation/create");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":elicitation["id"],
            "result":{"action":"decline"}
        })
    )
    .unwrap();
    stdin.flush().unwrap();
    let result: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let text = result["result"]["content"][0]["text"].as_str().unwrap();
    let denied: Value = serde_json::from_str(text).unwrap();
    assert_eq!(denied["status"], "denied");

    drop(stdin);
    assert!(child.wait().unwrap().success());
    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&staged.stdout).trim().is_empty());
}
