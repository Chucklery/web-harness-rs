use crate::command_output;
use crate::env;
use crate::path_policy;
use crate::redact;
use crate::workspace::Workspace;
use serde::Serialize;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use thiserror::Error;

pub const MAX_CONTEXT_LINES: usize = 2;
const MAX_CONTEXT_TEXT_CHARS: usize = 500;
pub const MAX_CONTEXT_OUTPUT_BYTES: usize = 128 * 1024;
const MAX_CONTEXT_RECORDS: usize = 8 * 1024;

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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<SearchContextLine>,
    #[serde(skip_serializing_if = "is_false")]
    pub context_truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchContextLine {
    pub line: u64,
    pub text: String,
}

fn is_false(value: &bool) -> bool {
    !value
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
    pub context_lines: usize,
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
            context_lines: 0,
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
    let context_lines = options.context_lines.min(MAX_CONTEXT_LINES);
    workspace
        .resolve(&options.scope)
        .map_err(|error| SearchError::Failed(error.to_string()))?;
    let mut command = Command::new("rg");
    command.args(["--color", "never"]);
    match options.mode {
        SearchMode::Matches => {
            command.args(["--json", "--line-number"]);
            if context_lines > 0 {
                command.arg("--context").arg(context_lines.to_string());
            }
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
        if path_policy::is_safe_example(&options.scope) {
            for include in path_policy::SAFE_EXAMPLE_GLOBS {
                command.args(["--glob", include]);
            }
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
            let (matches, more) = parse_matches(
                &stdout,
                max_results,
                options.offset,
                context_lines,
                output.stdout_truncated,
            );
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
fn parse_matches(
    stdout: &str,
    max_results: usize,
    offset: usize,
    context_lines: usize,
    context_output_truncated: bool,
) -> (Vec<SearchMatch>, bool) {
    let mut matches = Vec::new();
    let mut nearby_lines = HashMap::<String, HashMap<u64, String>>::new();
    let mut nearby_line_count = 0;
    let mut nearby_lines_truncated = false;
    let mut seen = 0;
    let mut more = false;
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let record_type = value.get("type").and_then(|value| value.as_str());
        if record_type != Some("match") && record_type != Some("context") {
            continue;
        }
        let data = &value["data"];
        let path = data["path"]["text"].as_str().unwrap_or_default();
        let line_number = data["line_number"].as_u64().unwrap_or_default();
        let raw_text = data["lines"]["text"].as_str().unwrap_or_default();
        let text = if record_type == Some("match") {
            raw_text.trim_end()
        } else {
            raw_text.trim_end_matches(['\r', '\n'])
        };
        if context_lines > 0 && !path.is_empty() && line_number > 0 {
            let file_lines = nearby_lines.entry(path.to_string()).or_default();
            if let std::collections::hash_map::Entry::Vacant(entry) = file_lines.entry(line_number)
            {
                if nearby_line_count < MAX_CONTEXT_RECORDS {
                    entry.insert(text.to_string());
                    nearby_line_count += 1;
                } else {
                    nearby_lines_truncated = true;
                }
            }
        }
        if record_type != Some("match") {
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
        matches.push(SearchMatch {
            path: path.to_string(),
            line: line_number,
            // Matched text is file content and can contain a secret literal;
            // scrub it here so the tool never returns it verbatim.
            text: redact::text(text).chars().take(2000).collect(),
            context: Vec::new(),
            context_truncated: false,
        });
        seen += 1;
    }
    if context_lines > 0 {
        attach_context(
            &mut matches,
            &nearby_lines,
            context_lines.min(MAX_CONTEXT_LINES),
            context_output_truncated || nearby_lines_truncated,
        );
    }
    (matches, more)
}

fn attach_context(
    matches: &mut [SearchMatch],
    nearby_lines: &HashMap<String, HashMap<u64, String>>,
    context_lines: usize,
    context_output_truncated: bool,
) {
    for matched in matches.iter_mut() {
        matched.context_truncated = context_output_truncated;
        let first_line = matched.line.saturating_sub(context_lines as u64).max(1);
        let last_line = matched.line.saturating_add(context_lines as u64);
        for line_number in first_line..=last_line {
            if line_number == matched.line {
                continue;
            }
            let Some(text) = nearby_lines
                .get(&matched.path)
                .and_then(|file_lines| file_lines.get(&line_number))
            else {
                continue;
            };
            let text = redact::text(text)
                .chars()
                .take(MAX_CONTEXT_TEXT_CHARS)
                .collect::<String>();
            matched.context.push(SearchContextLine {
                line: line_number,
                text,
            });
        }
    }
    let mut remaining_bytes = MAX_CONTEXT_OUTPUT_BYTES;
    limit_context_output(matches, &mut remaining_bytes);
}

pub(crate) fn limit_context_output(matches: &mut [SearchMatch], remaining_bytes: &mut usize) {
    for matched in matches {
        let mut bounded_context = Vec::with_capacity(matched.context.len());
        for line in matched.context.drain(..) {
            let encoded_bytes = serde_json::to_string(&line)
                .map(|encoded| encoded.len())
                .unwrap_or_else(|_| line.text.len());
            if encoded_bytes > *remaining_bytes {
                matched.context_truncated = true;
                continue;
            }
            *remaining_bytes -= encoded_bytes;
            bounded_context.push(line);
        }
        matched.context = bounded_context;
    }
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

    fn context_line(path: &str, line: u64, text: &str) -> String {
        serde_json::json!({
            "type": "context",
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
        let (matches, _) = parse_matches(&stdout, 10, 0, 0, false);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, "a.txt");
        assert_eq!(matches[0].line, 3);
        assert_eq!(matches[0].text, "API_KEY=[REDACTED]");
        assert!(!matches[1].text.contains("topsecret"));
    }

    #[test]
    fn nearby_context_is_bounded_and_redacted() {
        let stdout = format!(
            "{}\n{}\n{}\n",
            context_line("a.txt", 1, "before\n"),
            match_line("a.txt", 2, "needle\n"),
            context_line("a.txt", 3, "API_KEY=neighbor-secret\n"),
        );
        let (matches, _) = parse_matches(&stdout, 10, 0, 1, false);
        assert_eq!(matches[0].context.len(), 2);
        assert_eq!(matches[0].context[0].line, 1);
        assert_eq!(matches[0].context[0].text, "before");
        assert_eq!(matches[0].context[1].line, 3);
        assert_eq!(matches[0].context[1].text, "API_KEY=[REDACTED]");
        assert!(!matches[0].context[1].text.contains("neighbor-secret"));
    }

    #[test]
    fn context_output_has_a_shared_serialized_byte_budget() {
        let mut matches = (0..200)
            .map(|index| SearchMatch {
                path: "a.txt".into(),
                line: index * 10 + 5,
                text: "needle".into(),
                context: Vec::new(),
                context_truncated: false,
            })
            .collect::<Vec<_>>();
        let mut nearby_lines = HashMap::<String, HashMap<u64, String>>::new();
        for matched in &matches {
            for offset in [-2i64, -1, 1, 2] {
                nearby_lines
                    .entry(matched.path.clone())
                    .or_default()
                    .insert(
                        (matched.line as i64 + offset) as u64,
                        "x".repeat(MAX_CONTEXT_TEXT_CHARS),
                    );
            }
        }
        attach_context(&mut matches, &nearby_lines, 2, false);

        let encoded_bytes: usize = matches
            .iter()
            .flat_map(|matched| matched.context.iter())
            .map(|line| serde_json::to_string(&line.text).unwrap().len())
            .sum();
        assert!(encoded_bytes <= MAX_CONTEXT_OUTPUT_BYTES);
        assert!(matches.iter().any(|matched| matched.context_truncated));
    }

    #[test]
    fn non_match_records_and_malformed_lines_are_ignored() {
        let stdout = format!(
            "{{\"type\":\"begin\"}}\nnot json\n{}\n{{\"type\":\"summary\"}}\n",
            match_line("a.txt", 1, "needle\n"),
        );
        let (matches, _) = parse_matches(&stdout, 10, 0, 0, false);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn match_count_is_bounded() {
        let stdout = (1..=50)
            .map(|n| match_line("a.txt", n, "needle\n"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_matches(&stdout, 5, 0, 0, false).0.len(), 5);
    }

    #[test]
    fn match_continuation_skips_previous_results() {
        let stdout = (1..=3)
            .map(|n| match_line("a.txt", n, "needle\n"))
            .collect::<Vec<_>>()
            .join("\n");
        let (matches, more) = parse_matches(&stdout, 1, 1, 0, false);
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
    fn ripgrep_context_is_returned_with_the_match() {
        if !ripgrep_available() {
            eprintln!("skipping: ripgrep is not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        fs::write(dir.path().join("sample.txt"), "before\nneedle\nafter\n").unwrap();
        let options = SearchOptions {
            literal: true,
            context_lines: 1,
            ..SearchOptions::default()
        };

        let result = search(&workspace, "needle", 10, &options).unwrap();
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].context.len(), 2);
        assert_eq!(result.matches[0].context[0].line, 1);
        assert_eq!(result.matches[0].context[0].text, "before");
        assert_eq!(result.matches[0].context[1].line, 3);
        assert_eq!(result.matches[0].context[1].text, "after");
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
