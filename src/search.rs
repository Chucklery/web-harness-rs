use crate::workspace::Workspace;
use serde::Serialize;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("ripgrep is not available")]
    RipgrepUnavailable,
    #[error("search failed: {0}")]
    Failed(String),
}

#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: u64,
    pub text: String,
}

pub fn content_search(
    workspace: &Workspace,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchMatch>, SearchError> {
    let max_results = max_results.clamp(1, 200);
    let output = Command::new("rg")
        .args(["--json", "--line-number", "--color", "never", "--"])
        .arg(query)
        .arg(".")
        .current_dir(workspace.root())
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                SearchError::RipgrepUnavailable
            } else {
                SearchError::Failed(error.to_string())
            }
        })?;

    if !output.status.success() && output.status.code() != Some(1) {
        return Err(SearchError::Failed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|value| value.as_str()) != Some("match") {
            continue;
        }
        let data = &value["data"];
        matches.push(SearchMatch {
            path: data["path"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            line: data["line_number"].as_u64().unwrap_or_default(),
            text: data["lines"]["text"]
                .as_str()
                .unwrap_or_default()
                .trim_end()
                .chars()
                .take(2000)
                .collect(),
        });
        if matches.len() >= max_results {
            break;
        }
    }
    Ok(matches)
}
