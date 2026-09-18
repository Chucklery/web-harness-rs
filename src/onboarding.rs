use crate::config::{self, UserConfig};
use serde::Serialize;
use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const BLOCK_START: &str = "# >>> web-harness tunnel >>>";
const BLOCK_END: &str = "# <<< web-harness tunnel <<<";
const TUNNEL_ID_KEY: &str = "CONTROL_PLANE_TUNNEL_ID";
const API_KEY: &str = "CONTROL_PLANE_API_KEY";

#[derive(Debug)]
pub struct SetupOptions {
    pub tunnel_id: Option<String>,
    pub api_key: Option<String>,
    pub tunnel_command_json: Option<String>,
    pub tunnel_wrapper: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct SetupResult {
    pub workspace: String,
    pub config_path: String,
    pub shell_config_path: String,
    pub shell_backup_path: Option<String>,
    pub tunnel_wrapper: String,
    pub tunnel_id: String,
    pub api_key_configured: bool,
    pub tunnel_client_available: bool,
}

#[derive(Debug, Serialize)]
pub struct SetupStatus {
    pub configured: bool,
    pub workspace: Option<String>,
    pub tunnel_id: Option<String>,
    pub api_key_configured: bool,
    pub shell_config_path: String,
    pub tunnel_client_available: bool,
}

#[derive(Debug, Error)]
pub enum OnboardingError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(
        "tunnel id is invalid; expected tunnel_ followed by 32 lowercase hexadecimal characters"
    )]
    InvalidTunnelId,
    #[error("API key must be non-empty and must not contain newlines or NUL bytes")]
    InvalidApiKey,
    #[error("--api-key and --tunnel-id must be provided together in non-interactive mode")]
    IncompleteCredentials,
    #[error("--tunnel-command-json and --tunnel-wrapper cannot both be set")]
    ConflictingTunnelOverrides,
    #[error(
        "bundled OpenAI tunnel-client was not found; reinstall web-harness from the official GitHub Release or Homebrew package"
    )]
    TunnelClientUnavailable,
}

pub fn setup(workspace: &Path, options: SetupOptions) -> Result<SetupResult, OnboardingError> {
    if options.tunnel_command_json.is_some() && options.tunnel_wrapper.is_some() {
        return Err(OnboardingError::ConflictingTunnelOverrides);
    }

    let custom_tunnel = options.tunnel_command_json.is_some() || options.tunnel_wrapper.is_some();
    let credentials_requested =
        options.tunnel_id.is_some() || options.api_key.is_some() || !custom_tunnel;
    let tunnel_client = if custom_tunnel {
        resolve_tunnel_client()
    } else {
        Some(resolve_tunnel_client().ok_or(OnboardingError::TunnelClientUnavailable)?)
    };

    let (tunnel_id, api_key, shell_backup_path) = if credentials_requested {
        let interactive = io::stdin().is_terminal();
        if interactive && (options.tunnel_id.is_none() || options.api_key.is_none()) {
            eprintln!("OpenAI Platform: https://platform.openai.com/");
            eprintln!("Get tunnel_id: https://platform.openai.com/settings/organization/tunnels");
            eprintln!(
                "Create runtime API key: https://platform.openai.com/settings/organization/api-keys"
            );
            eprintln!(
                "warning: web-harness will store CONTROL_PLANE_TUNNEL_ID and CONTROL_PLANE_API_KEY as plaintext in {}",
                zshrc_path()?.display()
            );
            eprintln!("the managed .zshrc file is written with owner-only permissions (0600)");
        }
        let tunnel_id = match options.tunnel_id {
            Some(value) => value,
            None if interactive => prompt_line("OpenAI tunnel_id: ")?,
            None => return Err(OnboardingError::IncompleteCredentials),
        };
        let api_key = match options.api_key {
            Some(value) => value,
            None if interactive => prompt_secret("OpenAI runtime API key: ")?,
            None => return Err(OnboardingError::IncompleteCredentials),
        };
        validate_tunnel_id(&tunnel_id)?;
        validate_api_key(&api_key)?;
        let backup = persist_zsh_credentials(&tunnel_id, &api_key)?;
        (tunnel_id, api_key, backup)
    } else {
        let env = managed_shell_env()?;
        (env.tunnel_id.unwrap_or_default(), String::new(), None)
    };

    let user_config = if let Some(wrapper) = options.tunnel_wrapper {
        UserConfig::from_wrapper(workspace, &wrapper)?
    } else if let Some(command) = options.tunnel_command_json {
        UserConfig::from_inputs(workspace, Some(&command))?
    } else {
        let wrapper = generate_tunnel_wrapper()?;
        UserConfig::from_wrapper(workspace, &wrapper)?
    };
    let config_path = config::save(&user_config)?;
    let wrapper_path = user_config
        .tunnel_command
        .as_ref()
        .and_then(|argv| argv.first())
        .cloned()
        .unwrap_or_default();

    Ok(SetupResult {
        workspace: workspace.display().to_string(),
        config_path: config_path.display().to_string(),
        shell_config_path: zshrc_path()?.display().to_string(),
        shell_backup_path: shell_backup_path.map(|path| path.display().to_string()),
        tunnel_wrapper: wrapper_path,
        tunnel_id,
        api_key_configured: !api_key.is_empty() || managed_shell_env()?.api_key.is_some(),
        tunnel_client_available: tunnel_client.is_some(),
    })
}

pub fn status() -> Result<SetupStatus, OnboardingError> {
    let user_config = config::load_optional()?;
    let shell = managed_shell_env()?;
    Ok(SetupStatus {
        configured: user_config.is_some(),
        workspace: user_config.map(|value| value.workspace),
        tunnel_id: shell.tunnel_id,
        api_key_configured: shell.api_key.is_some(),
        shell_config_path: zshrc_path()?.display().to_string(),
        tunnel_client_available: resolve_tunnel_client().is_some(),
    })
}

pub fn format_setup_result(result: &SetupResult) -> String {
    let mut lines = vec![
        "Configured web-harness.".to_string(),
        format!("workspace: {}", result.workspace),
        format!("config: {}", result.config_path),
        format!("shell config: {}", result.shell_config_path),
        "credential storage: plaintext managed .zshrc block (mode 0600)".to_string(),
        format!("tunnel_id: {}", result.tunnel_id),
        format!(
            "api_key: {}",
            if result.api_key_configured {
                "configured (hidden)"
            } else {
                "not managed by web-harness"
            }
        ),
        format!("tunnel wrapper: {}", result.tunnel_wrapper),
    ];
    if let Some(backup) = &result.shell_backup_path {
        lines.push(format!("shell backup: {backup}"));
    }
    if result.tunnel_client_available {
        lines.push("tunnel-client: bundled/available".into());
        lines.push("next: web-harness connect".into());
    } else {
        lines.push("tunnel-client: unavailable; reinstall web-harness".into());
    }
    lines.join("\n")
}

pub fn format_setup_status(status: &SetupStatus) -> String {
    [
        format!(
            "configured: {}",
            if status.configured { "yes" } else { "no" }
        ),
        format!("workspace: {}", status.workspace.as_deref().unwrap_or("-")),
        format!("tunnel_id: {}", status.tunnel_id.as_deref().unwrap_or("-")),
        format!(
            "api_key: {}",
            if status.api_key_configured {
                "configured (hidden)"
            } else {
                "not configured"
            }
        ),
        format!("shell config: {}", status.shell_config_path),
        format!(
            "tunnel-client: {}",
            if status.tunnel_client_available {
                "bundled/available"
            } else {
                "not found"
            }
        ),
    ]
    .join("\n")
}

#[derive(Debug, Default)]
pub struct ManagedShellEnv {
    pub tunnel_id: Option<String>,
    pub api_key: Option<String>,
}

pub fn managed_shell_env() -> Result<ManagedShellEnv, OnboardingError> {
    let path = zshrc_path()?;
    managed_shell_env_at(&path)
}

fn managed_shell_env_at(path: &Path) -> Result<ManagedShellEnv, OnboardingError> {
    if !path.exists() {
        return Ok(ManagedShellEnv::default());
    }
    let text = fs::read_to_string(path)?;
    let Some(block) = extract_managed_block(&text) else {
        return Ok(ManagedShellEnv::default());
    };
    Ok(ManagedShellEnv {
        tunnel_id: extract_export(block, TUNNEL_ID_KEY),
        api_key: extract_export(block, API_KEY),
    })
}

fn persist_zsh_credentials(
    tunnel_id: &str,
    api_key: &str,
) -> Result<Option<PathBuf>, OnboardingError> {
    let path = zshrc_path()?;
    persist_zsh_credentials_at(&path, tunnel_id, api_key)
}

fn persist_zsh_credentials_at(
    path: &Path,
    tunnel_id: &str,
    api_key: &str,
) -> Result<Option<PathBuf>, OnboardingError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let backup = if path.exists() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let backup = parent.join(format!(
            "{}.web-harness.bak.{suffix}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(".zshrc")
        ));
        let safe_backup = remove_managed_block(&existing);
        atomic_write_private(&backup, safe_backup.as_bytes())?;
        Some(backup)
    } else {
        None
    };

    let block = format!(
        "{BLOCK_START}\nexport {TUNNEL_ID_KEY}={}\nexport {API_KEY}={}\n{BLOCK_END}",
        shell_quote(tunnel_id),
        shell_quote(api_key)
    );
    let updated = replace_managed_block(&existing, &block);
    atomic_write_private(&path, updated.as_bytes())?;
    Ok(backup)
}

fn generate_tunnel_wrapper() -> Result<PathBuf, OnboardingError> {
    let config_path = config::config_path()?;
    let dir = config_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid config path"))?;
    fs::create_dir_all(dir)?;
    let path = dir.join("tunnel-wrapper.zsh");
    let script = tunnel_wrapper_script();
    atomic_write_private(&path, script.as_bytes())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

fn tunnel_wrapper_script() -> &'static str {
    r#"#!/bin/zsh
set -eu

: "${CONTROL_PLANE_TUNNEL_ID:?CONTROL_PLANE_TUNNEL_ID is required}"
: "${CONTROL_PLANE_API_KEY:?CONTROL_PLANE_API_KEY is required}"
: "${WEB_HARNESS_SERVER_BIN:?WEB_HARNESS_SERVER_BIN is required}"
: "${WEB_HARNESS_WORKSPACE:?WEB_HARNESS_WORKSPACE is required}"
: "${WEB_HARNESS_TUNNEL_CLIENT_BIN:?bundled tunnel-client path is required}"

mcp_command="$(printf '%q' "$WEB_HARNESS_SERVER_BIN") serve --stdio --workspace $(printf '%q' "$WEB_HARNESS_WORKSPACE")"

exec "$WEB_HARNESS_TUNNEL_CLIENT_BIN" run --mcp.command="$mcp_command"
"#
}

fn prompt_line(prompt: &str) -> Result<String, OnboardingError> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_string())
}

fn prompt_secret(prompt: &str) -> Result<String, OnboardingError> {
    print!("{prompt}");
    io::stdout().flush()?;
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let original = unsafe {
        let mut termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) != 0 {
            return Err(io::Error::last_os_error().into());
        }
        termios
    };
    let mut hidden = original;
    hidden.c_lflag &= !libc::ECHO;
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
        return Err(io::Error::last_os_error().into());
    }

    struct EchoGuard {
        fd: i32,
        original: libc::termios,
    }
    impl Drop for EchoGuard {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
            }
        }
    }
    let _guard = EchoGuard { fd, original };
    let mut value = String::new();
    let result = stdin.read_line(&mut value);
    println!();
    result?;
    Ok(value.trim().to_string())
}

fn validate_tunnel_id(value: &str) -> Result<(), OnboardingError> {
    let suffix = value
        .strip_prefix("tunnel_")
        .ok_or(OnboardingError::InvalidTunnelId)?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(OnboardingError::InvalidTunnelId);
    }
    Ok(())
}

fn validate_api_key(value: &str) -> Result<(), OnboardingError> {
    if value.trim().is_empty() || value.contains(['\n', '\r', '\0']) {
        return Err(OnboardingError::InvalidApiKey);
    }
    Ok(())
}

fn zshrc_path() -> Result<PathBuf, OnboardingError> {
    if let Some(zdotdir) = std::env::var_os("ZDOTDIR") {
        return Ok(PathBuf::from(zdotdir).join(".zshrc"));
    }
    let home = std::env::var_os("HOME").ok_or(config::ConfigError::MissingHome)?;
    Ok(PathBuf::from(home).join(".zshrc"))
}

fn replace_managed_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end_start)) = (existing.find(BLOCK_START), existing.find(BLOCK_END)) {
        if end_start >= start {
            let end = end_start + BLOCK_END.len();
            let mut output = String::with_capacity(existing.len() + block.len());
            output.push_str(&existing[..start]);
            output.push_str(block);
            output.push_str(&existing[end..]);
            return output;
        }
    }
    let mut output = existing.trim_end_matches('\n').to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(block);
    output.push('\n');
    output
}

fn remove_managed_block(existing: &str) -> String {
    if let (Some(start), Some(end_start)) = (existing.find(BLOCK_START), existing.find(BLOCK_END)) {
        if end_start >= start {
            let end = end_start + BLOCK_END.len();
            let mut output = String::with_capacity(existing.len());
            output.push_str(&existing[..start]);
            output.push_str(&existing[end..]);
            return output;
        }
    }
    existing.to_string()
}

fn extract_managed_block(text: &str) -> Option<&str> {
    let start = text.find(BLOCK_START)?;
    let tail = &text[start..];
    let end = tail.find(BLOCK_END)? + BLOCK_END.len();
    Some(&tail[..end])
}

fn extract_export(block: &str, key: &str) -> Option<String> {
    let prefix = format!("export {key}=");
    block
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(shell_unquote)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_unquote(value: &str) -> Option<String> {
    if !value.starts_with('\'') || !value.ends_with('\'') {
        return None;
    }
    Some(value[1..value.len() - 1].replace("'\\''", "'"))
}

fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), io::Error> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.as_file_mut().write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.as_file_mut()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn resolve_tunnel_client() -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os("WEB_HARNESS_TUNNEL_CLIENT_BIN") {
        let path = PathBuf::from(override_path);
        if is_executable(&path) {
            return Some(path);
        }
    }
    let executable = std::env::current_exe().ok()?;
    resolve_tunnel_client_from(&executable, std::env::var_os("PATH"))
}

fn resolve_tunnel_client_from(executable: &Path, path_env: Option<OsString>) -> Option<PathBuf> {
    let executable_dir = executable.parent()?;
    let mut candidates = vec![
        executable_dir.join("libexec/web-harness/tunnel-client"),
        executable_dir.join("../libexec/web-harness/tunnel-client"),
    ];
    if let Some(path) = path_env {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join("tunnel-client")));
    }
    candidates
        .into_iter()
        .find(|candidate| is_executable(candidate))
}

fn is_executable(candidate: &Path) -> bool {
    candidate.is_file()
        && candidate
            .metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_tunnel_id() {
        validate_tunnel_id("tunnel_0123456789abcdef0123456789abcdef").unwrap();
        assert!(validate_tunnel_id("tunnel_bad").is_err());
    }

    #[test]
    fn validates_api_key_input_shape() {
        validate_api_key("dummy-runtime-key").unwrap();
        assert!(validate_api_key("").is_err());
        assert!(validate_api_key("bad\nkey").is_err());
        assert!(validate_api_key("bad\0key").is_err());
    }

    #[test]
    fn managed_block_is_idempotent_and_updates_values() {
        let first = replace_managed_block(
            "export PATH=/bin\n",
            &format!(
                "{BLOCK_START}\nexport {TUNNEL_ID_KEY}='tunnel_0123456789abcdef0123456789abcdef'\nexport {API_KEY}='one'\n{BLOCK_END}"
            ),
        );
        let second = replace_managed_block(
            &first,
            &format!(
                "{BLOCK_START}\nexport {TUNNEL_ID_KEY}='tunnel_fedcba9876543210fedcba9876543210'\nexport {API_KEY}='two'\n{BLOCK_END}"
            ),
        );
        assert_eq!(second.matches(BLOCK_START).count(), 1);
        assert!(!second.contains("'one'"));
        assert!(second.contains("'two'"));
    }

    #[test]
    fn shell_quote_round_trips() {
        let value = "sk-test'quote";
        assert_eq!(shell_unquote(&shell_quote(value)).unwrap(), value);
    }

    #[test]
    fn generated_config_does_not_need_secret_fields() {
        let dir = tempfile::tempdir().unwrap();
        let wrapper = dir.path().join("wrapper");
        fs::write(&wrapper, "").unwrap();
        let config = UserConfig::from_wrapper(dir.path(), &wrapper).unwrap();
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("api_key"));
        assert!(!json.contains("CONTROL_PLANE_API_KEY"));
    }

    #[test]
    fn zshrc_update_is_idempotent_and_creates_private_backup() {
        let dir = tempfile::tempdir().unwrap();
        let zshrc = dir.path().join(".zshrc");
        fs::write(&zshrc, "export PATH=/usr/bin\n").unwrap();

        let first = persist_zsh_credentials_at(
            &zshrc,
            "tunnel_0123456789abcdef0123456789abcdef",
            "sk-test-one",
        )
        .unwrap();
        assert!(first.unwrap().exists());

        let second = persist_zsh_credentials_at(
            &zshrc,
            "tunnel_fedcba9876543210fedcba9876543210",
            "sk-test-two",
        )
        .unwrap();
        let backup = second.unwrap();
        assert!(backup.exists());
        assert_eq!(
            fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let backup_text = fs::read_to_string(&backup).unwrap();
        assert!(backup_text.contains("export PATH=/usr/bin"));
        assert!(!backup_text.contains("sk-test-one"));
        assert!(!backup_text.contains(BLOCK_START));

        let text = fs::read_to_string(&zshrc).unwrap();
        assert_eq!(
            fs::metadata(&zshrc).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(text.matches(BLOCK_START).count(), 1);
        assert!(text.contains("tunnel_fedcba9876543210fedcba9876543210"));
        assert!(text.contains("sk-test-two"));
        assert!(!text.contains("sk-test-one"));

        let parsed = managed_shell_env_at(&zshrc).unwrap();
        assert_eq!(
            parsed.tunnel_id.as_deref(),
            Some("tunnel_fedcba9876543210fedcba9876543210")
        );
        assert_eq!(parsed.api_key.as_deref(), Some("sk-test-two"));
    }

    #[test]
    fn generated_wrapper_references_env_not_secret_literal() {
        let script = tunnel_wrapper_script();
        assert!(script.contains("CONTROL_PLANE_API_KEY"));
        assert!(script.contains("CONTROL_PLANE_TUNNEL_ID"));
        assert!(script.contains("WEB_HARNESS_TUNNEL_CLIENT_BIN"));
        assert!(script.contains("--mcp.command"));
        assert!(!script.contains("sk-"));
    }

    #[test]
    fn resolves_release_archive_bundled_tunnel_client() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("web-harness");
        fs::write(&executable, "").unwrap();
        let bundled = dir.path().join("libexec/web-harness/tunnel-client");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&bundled, "").unwrap();
        fs::set_permissions(&bundled, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            resolve_tunnel_client_from(&executable, None).unwrap(),
            bundled
        );
    }

    #[test]
    fn resolves_homebrew_bundled_tunnel_client() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("bin/web-harness");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "").unwrap();
        let bundled = dir.path().join("libexec/web-harness/tunnel-client");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&bundled, "").unwrap();
        fs::set_permissions(&bundled, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            resolve_tunnel_client_from(&executable, None)
                .unwrap()
                .canonicalize()
                .unwrap(),
            bundled.canonicalize().unwrap()
        );
    }
}
