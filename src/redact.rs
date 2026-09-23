const SECRET_KEYS: &[&str] = &[
    "authorization",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "token",
    "password",
    "passwd",
    "secret",
    "cookie",
    "set-cookie",
    "private_key",
];

pub fn text(input: &str) -> String {
    let mut output = input
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n");
    if input.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn redact_line(line: &str) -> String {
    let mut out = line.to_string();
    for key in SECRET_KEYS {
        out = redact_assignment(&out, key, '=');
        out = redact_assignment(&out, key, ':');
    }
    redact_bearer(&out)
}

fn redact_assignment(input: &str, key: &str, separator: char) -> String {
    let lower = input.to_ascii_lowercase();
    let pattern = format!("{key}{separator}");
    let mut cursor = 0usize;
    let mut output = String::with_capacity(input.len());
    while let Some(offset) = lower[cursor..].find(&pattern) {
        let start = cursor + offset;
        let value_start = start + pattern.len();
        output.push_str(&input[cursor..value_start]);
        output.push_str("[REDACTED]");
        let value_end = input[value_start..]
            .find(|c: char| c.is_whitespace() || c == '&' || c == ',' || c == ';')
            .map(|index| value_start + index)
            .unwrap_or(input.len());
        cursor = value_end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn redact_bearer(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let Some(start) = lower.find("bearer ") else {
        return input.to_string();
    };
    let value_start = start + "bearer ".len();
    let value_end = input[value_start..]
        .find(char::is_whitespace)
        .map(|index| value_start + index)
        .unwrap_or(input.len());
    format!("{}[REDACTED]{}", &input[..value_start], &input[value_end..])
}

pub fn sensitive_env_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|needle| lower.contains(needle))
        || lower.ends_with("_key")
        || lower.ends_with("_credential")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_assignments_and_bearer_tokens() {
        let input = "API_KEY=abc123 password:hello Authorization: Bearer topsecret";
        let output = text(input);
        assert!(!output.contains("abc123"));
        assert!(!output.contains("hello"));
        assert!(!output.contains("topsecret"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn preserves_trailing_newlines_in_redacted_output() {
        assert_eq!(text("plain\n"), "plain\n");
        assert_eq!(text("TOKEN=secret\n"), "TOKEN=[REDACTED]\n");
        assert_eq!(text("first\n\n"), "first\n\n");
        assert_eq!(text("unterminated"), "unterminated");
    }

    #[test]
    fn detects_sensitive_environment_keys() {
        assert!(sensitive_env_key("OPENAI_API_KEY"));
        assert!(sensitive_env_key("GITHUB_TOKEN"));
        assert!(sensitive_env_key("SESSION_COOKIE"));
        assert!(!sensitive_env_key("PATH"));
    }
}
