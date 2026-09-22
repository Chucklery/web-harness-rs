use crate::command_output;
use crate::env;
use crate::path_policy;
use crate::redact;
use crate::workspace::Workspace;
use serde::Serialize;
use std::process::{Command, Stdio};
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

/// Reports whether ripgrep can actually be executed from the bounded
/// environment the search tool uses.
///
/// Probing here rather than letting the first `search` call fail means a client
/// can learn the dependency is missing before issuing a request that cannot
/// succeed. The probe deliberately reuses [`env::apply`] so it observes the same
/// PATH the real search will.
pub fn ripgrep_available() -> bool {
    let mut command = Command::new("rg");
    command.arg("--version");
    env::apply(&mut command);
    matches!(command.output(), Ok(output) if output.status.success())
}

#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Matches,
    FilesWithMatches,
    Count,
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub literal: bool,
    pub scope: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub mode: SearchMode,
    pub offset: usize,
    pub allow_protected: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            literal: false,
            scope: ".".into(),
            include: Vec::new(),
            exclude: Vec::new(),
            mode: SearchMode::Matches,
            offset: 0,
            allow_protected: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchCount {
    pub path: String,
    pub count: u64,
}

#[derive(Debug, Serialize)]
pub struct SearchOutput {
    pub matches: Vec<SearchMatch>,
    pub files: Vec<String>,
    pub counts: Vec<SearchCount>,
    pub truncated: bool,
    pub offset: usize,
    pub next_offset: Option<usize>,
}

pub fn content_search(
    workspace: &Workspace,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchMatch>, SearchError> {
    let output = search(workspace, query, max_results, &SearchOptions::default())?;
    Ok(output.matches)
}

pub fn search(
    workspace: &Workspace,
    query: &str,
    max_results: usize,
    options: &SearchOptions,
) -> Result<SearchOutput, SearchError> {
    let max_results = max_results.clamp(1, 200);
    workspace
        .resolve(&options.scope)
        .map_err(|error| SearchError::Failed(error.to_string()))?;
    let mut command = Command::new("rg");
    command.args(["--color", "never"]);
    match options.mode {
        SearchMode::Matches => {
            command.args(["--json", "--line-number"]);
        }
        SearchMode::FilesWithMatches => {
            command.arg("--files-with-matches");
        }
        SearchMode::Count => {
            command.args(["--count", "--no-heading"]);
        }
    }
    if options.literal {
        command.arg("--fixed-strings");
    }
    for include in &options.include {
        command.args(["--glob", include]);
    }
    for exclude in &options.exclude {
        command.args(["--glob", &format!("!{exclude}")]);
    }
    if !options.allow_protected {
        for exclude in path_policy::PROTECTED_GLOBS {
            command.args(["--glob", exclude]);
        }
        for include in path_policy::SAFE_EXAMPLE_GLOBS {
            command.args(["--glob", include]);
        }
        for exclude in path_policy::PROTECTED_DIRECTORY_GLOBS {
            command.args(["--glob", exclude]);
        }
    }
    command
        .arg("--")
        .arg(query)
        .arg(&options.scope)
        .current_dir(workspace.root());
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Search runs outside the execution sandbox, but it must still start from a
    // bounded environment: an inherited `RIPGREP_CONFIG_PATH` or credential
    // variable would otherwise reach the child unfiltered.
    env::apply(&mut command);
    let child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SearchError::RipgrepUnavailable
        } else {
            SearchError::Failed(error.to_string())
        }
    })?;
    let output = command_output::collect(child, 2 * 1024 * 1024, 64 * 1024)
        .map_err(|error| SearchError::Failed(error.to_string()))?;

    if !output.status.success() && output.status.code() != Some(1) {
        return Err(SearchError::Failed(redact::text(
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut result = SearchOutput {
        matches: Vec::new(),
        files: Vec::new(),
        counts: Vec::new(),
        truncated: output.stdout_truncated || output.stderr_truncated,
        offset: options.offset,
        next_offset: None,
    };
    match options.mode {
        SearchMode::Matches => {
            let (matches, more) = parse_matches(&stdout, max_results, options.offset);
            result.matches = matches;
            if more && !output.stdout_truncated {
                result.next_offset = Some(options.offset + result.matches.len());
            }
        }
        SearchMode::FilesWithMatches => {
            result.files = stdout
                .lines()
                .skip(options.offset)
                .take(max_results + 1)
                .map(|path| path.to_string())
                .collect();
            if result.files.len() > max_results {
                result.files.truncate(max_results);
                if !output.stdout_truncated {
                    result.next_offset = Some(options.offset + result.files.len());
                }
            }
        }
        SearchMode::Count => {
            for line in stdout.lines().skip(options.offset).take(max_results + 1) {
                let Some((path, count)) = line.rsplit_once(':') else {
                    continue;
                };
                if let Ok(count) = count.parse() {
                    result.counts.push(SearchCount {
                        path: path.to_string(),
                        count,
                    });
                }
            }
            if result.counts.len() > max_results {
                result.counts.truncate(max_results);
                if !output.stdout_truncated {
                    result.next_offset = Some(options.offset + result.counts.len());
                }
            }
        }
    }
    result.truncated |= result.next_offset.is_some();
    Ok(result)
}

/// Converts ripgrep's `--json` stream into at most `max_results` matches.
///
/// Split out from [`content_search`] so the redaction of matched text can be
/// tested without requiring ripgrep to be installed.
fn parse_matches(stdout: &str, max_results: usize, offset: usize) -> (Vec<SearchMatch>, bool) {
    let mut matches = Vec::new();
    let mut seen = 0;
    let mut more = false;
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(|value| value.as_str()) != Some("match") {
            continue;
        }
        if seen < offset {
            seen += 1;
            continue;
        }
        if matches.len() >= max_results {
            more = true;
            break;
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
        seen += 1;
    }
    (matches, more)
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
        let (matches, _) = parse_matches(&stdout, 10, 0);
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
        let (matches, _) = parse_matches(&stdout, 10, 0);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn match_count_is_bounded() {
        let stdout = (1..=50)
            .map(|n| match_line("a.txt", n, "needle\n"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_matches(&stdout, 5, 0).0.len(), 5);
    }

    #[test]
    fn match_continuation_skips_previous_results() {
        let stdout = (1..=3)
            .map(|n| match_line("a.txt", n, "needle\n"))
            .collect::<Vec<_>>()
            .join("\n");
        let (matches, more) = parse_matches(&stdout, 1, 1);
        assert_eq!(matches[0].line, 2);
        assert!(more);
    }

    #[test]
    fn default_options_preserve_match_mode() {
        assert_eq!(SearchOptions::default().mode, SearchMode::Matches);
    }

    #[test]
    fn safe_env_example_scope_is_not_filtered_as_protected() {
        if !ripgrep_available() {
            eprintln!("skipping: ripgrep is not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        fs::write(dir.path().join(".env.example"), "EXAMPLE=true\n").unwrap();
        let options = SearchOptions {
            scope: ".env.example".into(),
            ..SearchOptions::default()
        };
        let result = search(&workspace, "EXAMPLE", 10, &options).unwrap();
        assert_eq!(result.matches.len(), 1);
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
