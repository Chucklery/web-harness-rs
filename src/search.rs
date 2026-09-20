use crate::env;
use crate::redact;
use crate::workspace::Workspace;
use serde::Serialize;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error(
        "ripgrep is required by the search tool but was not found in PATH; install ripgrep and restart web-harness (macOS/Homebrew: `brew install ripgrep`, Debian/Ubuntu: `apt install ripgrep`)"
    )]
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
    let mut command = Command::new("rg");
    command
        .args(["--json", "--line-number", "--color", "never", "--"])
        .arg(query)
        .arg(".")
        .current_dir(workspace.root());
    // Search runs outside the execution sandbox, but it must still start from a
    // bounded environment: an inherited `RIPGREP_CONFIG_PATH` or credential
    // variable would otherwise reach the child unfiltered.
    env::apply(&mut command);
    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SearchError::RipgrepUnavailable
        } else {
            SearchError::Failed(error.to_string())
        }
    })?;

    if !output.status.success() && output.status.code() != Some(1) {
        return Err(SearchError::Failed(redact::text(
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }

    Ok(parse_matches(
        &String::from_utf8_lossy(&output.stdout),
        max_results,
    ))
}

/// Converts ripgrep's `--json` stream into at most `max_results` matches.
///
/// Split out from [`content_search`] so the redaction of matched text can be
/// tested without requiring ripgrep to be installed.
fn parse_matches(stdout: &str, max_results: usize) -> Vec<SearchMatch> {
    let mut matches = Vec::new();
    for line in stdout.lines() {
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
            // Matched text is file content and can contain a secret literal;
            // scrub it here so the tool never returns it verbatim.
            text: redact::text(
                data["lines"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_end(),
            )
            .chars()
            .take(2000)
            .collect(),
        });
        if matches.len() >= max_results {
            break;
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// `search` shells out to system ripgrep, which is a runtime dependency
    /// rather than a build dependency. Tests that need it skip when it is
    /// absent instead of failing, so `cargo test` stays meaningful on a machine
    /// that has not installed it yet.
    fn ripgrep_available() -> bool {
        Command::new("rg")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// A synthetic ripgrep `--json` line, matching the real schema.
    fn match_line(path: &str, line: u64, text: &str) -> String {
        serde_json::json!({
            "type": "match",
            "data": {
                "path": { "text": path },
                "line_number": line,
                "lines": { "text": text },
            }
        })
        .to_string()
    }

    #[test]
    fn matched_text_is_redacted() {
        let stdout = format!(
            "{}\n{}\n",
            match_line("a.txt", 3, "API_KEY=abc123\n"),
            match_line("b.txt", 9, "Authorization: Bearer topsecret\n"),
        );
        let matches = parse_matches(&stdout, 10);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, "a.txt");
        assert_eq!(matches[0].line, 3);
        assert_eq!(matches[0].text, "API_KEY=[REDACTED]");
        assert!(!matches[1].text.contains("topsecret"));
    }

    #[test]
    fn non_match_records_and_malformed_lines_are_ignored() {
        let stdout = format!(
            "{{\"type\":\"begin\"}}\nnot json\n{}\n{{\"type\":\"summary\"}}\n",
            match_line("a.txt", 1, "needle\n"),
        );
        let matches = parse_matches(&stdout, 10);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn match_count_is_bounded() {
        let stdout = (1..=50)
            .map(|n| match_line("a.txt", n, "needle\n"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_matches(&stdout, 5).len(), 5);
    }

    #[test]
    fn child_environment_is_minimal() {
        if !ripgrep_available() {
            eprintln!("skipping: ripgrep is not installed");
            return;
        }
        // `RIPGREP_CONFIG_PATH` would let an inherited variable change what the
        // child searches; the allowlist must drop it.
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        fs::write(dir.path().join("hit.txt"), "needle\n").unwrap();
        let config = dir.path().join("rg.conf");
        fs::write(&config, "--files\n").unwrap();
        std::env::set_var("RIPGREP_CONFIG_PATH", &config);
        let matches = content_search(&ws, "needle", 10);
        std::env::remove_var("RIPGREP_CONFIG_PATH");
        let matches = matches.unwrap();
        assert!(
            !matches.is_empty(),
            "inherited RIPGREP_CONFIG_PATH changed the search behaviour"
        );
    }
}
