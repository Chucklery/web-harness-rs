use crate::redact;
use crate::sandbox;
use crate::workspace::{Workspace, WorkspaceError};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_BACKGROUND_JOBS: usize = 2;
const OUTPUT_TAIL_BYTES: usize = 256 * 1024;
const MAX_TIMEOUT: Duration = Duration::from_secs(600);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Error)]
pub enum JobError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("background job limit reached")]
    Limit,
    #[error("job not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Serialize)]
pub struct ExecResult {
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
}

#[derive(Debug, Serialize)]
pub struct JobStatus {
    pub id: String,
    pub state: String,
    pub exit_code: Option<i32>,
}

struct Job {
    child: Child,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    exit_code: Option<i32>,
}

pub struct JobManager {
    jobs: HashMap<String, Job>,
}

impl JobManager {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    pub fn run_foreground(
        &self,
        workspace: &Workspace,
        argv: &[String],
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        sandboxed: bool,
    ) -> Result<ExecResult, JobError> {
        validate_argv(argv)?;
        let cwd = resolve_cwd(workspace, cwd)?;
        let (stdout_path, stdout) = create_artifact("stdout")?;
        let (stderr_path, stderr) = create_artifact("stderr")?;
        let effective_argv = if sandboxed {
            sandbox::wrap_argv(workspace, argv)
        } else {
            argv.to_vec()
        };
        let mut child = spawn(&effective_argv, &cwd, stdout, stderr)?;
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(120_000)).min(MAX_TIMEOUT);
        let start = Instant::now();
        let (exit_code, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (status.code(), false);
            }
            if start.elapsed() >= timeout {
                terminate(&mut child)?;
                let status = child.wait()?;
                break (status.code(), true);
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let (stdout_tail, stdout_truncated) = read_tail(&stdout_path, OUTPUT_TAIL_BYTES)?;
        let (stderr_tail, stderr_truncated) = read_tail(&stderr_path, OUTPUT_TAIL_BYTES)?;
        let _ = fs::remove_file(stdout_path);
        let _ = fs::remove_file(stderr_path);
        Ok(ExecResult {
            exit_code,
            stdout_tail: redact::text(&stdout_tail),
            stderr_tail: redact::text(&stderr_tail),
            stdout_truncated,
            stderr_truncated,
            timed_out,
        })
    }

    pub fn start(
        &mut self,
        workspace: &Workspace,
        argv: &[String],
        cwd: Option<&str>,
        sandboxed: bool,
    ) -> Result<JobStatus, JobError> {
        validate_argv(argv)?;
        self.refresh();
        if self
            .jobs
            .values()
            .filter(|job| job.exit_code.is_none())
            .count()
            >= MAX_BACKGROUND_JOBS
        {
            return Err(JobError::Limit);
        }
        let cwd = resolve_cwd(workspace, cwd)?;
        let (stdout_path, stdout) = create_artifact("stdout")?;
        let (stderr_path, stderr) = create_artifact("stderr")?;
        let effective_argv = if sandboxed {
            sandbox::wrap_argv(workspace, argv)
        } else {
            argv.to_vec()
        };
        let child = spawn(&effective_argv, &cwd, stdout, stderr)?;
        let id = format!(
            "job_{}_{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        self.jobs.insert(
            id.clone(),
            Job {
                child,
                stdout_path,
                stderr_path,
                exit_code: None,
            },
        );
        Ok(JobStatus {
            id,
            state: "running".into(),
            exit_code: None,
        })
    }

    pub fn poll(&mut self, id: &str) -> Result<JobStatus, JobError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| JobError::NotFound(id.to_string()))?;
        if job.exit_code.is_none() {
            if let Some(status) = job.child.try_wait()? {
                job.exit_code = status.code().or(Some(-1));
            }
        }
        Ok(JobStatus {
            id: id.to_string(),
            state: if job.exit_code.is_some() {
                "exited".into()
            } else {
                "running".into()
            },
            exit_code: job.exit_code,
        })
    }

    pub fn cancel(&mut self, id: &str) -> Result<JobStatus, JobError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| JobError::NotFound(id.to_string()))?;
        if job.exit_code.is_none() {
            terminate(&mut job.child)?;
            let status = job.child.wait()?;
            job.exit_code = status.code().or(Some(-1));
        }
        Ok(JobStatus {
            id: id.to_string(),
            state: "cancelled".into(),
            exit_code: job.exit_code,
        })
    }

    pub fn output(&self, id: &str, stream: &str) -> Result<(String, bool), JobError> {
        let job = self
            .jobs
            .get(id)
            .ok_or_else(|| JobError::NotFound(id.to_string()))?;
        let path = match stream {
            "stdout" => &job.stdout_path,
            "stderr" => &job.stderr_path,
            _ => return Err(JobError::Invalid("stream must be stdout or stderr".into())),
        };
        let (text, truncated) = read_tail(path, OUTPUT_TAIL_BYTES)?;
        Ok((redact::text(&text), truncated))
    }

    fn refresh(&mut self) {
        for job in self.jobs.values_mut() {
            if job.exit_code.is_none() {
                if let Ok(Some(status)) = job.child.try_wait() {
                    job.exit_code = status.code().or(Some(-1));
                }
            }
        }
    }
}

impl Drop for JobManager {
    fn drop(&mut self) {
        for job in self.jobs.values_mut() {
            if job.exit_code.is_none() {
                let _ = terminate(&mut job.child);
                let _ = job.child.wait();
            }
            let _ = fs::remove_file(&job.stdout_path);
            let _ = fs::remove_file(&job.stderr_path);
        }
    }
}

fn validate_argv(argv: &[String]) -> Result<(), JobError> {
    if argv.is_empty() || argv.len() > 64 {
        return Err(JobError::Invalid("argv must contain 1..=64 items".into()));
    }
    if argv.iter().any(|arg| arg.len() > 16 * 1024) {
        return Err(JobError::Invalid("argument exceeds 16 KiB".into()));
    }
    Ok(())
}

fn resolve_cwd(workspace: &Workspace, cwd: Option<&str>) -> Result<PathBuf, JobError> {
    match cwd {
        Some(cwd) => Ok(workspace.resolve(cwd)?),
        None => Ok(workspace.root().to_path_buf()),
    }
}

fn create_artifact(stream: &str) -> Result<(PathBuf, File), std::io::Error> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "web-harness-{}-{}-{}.log",
        std::process::id(),
        id,
        stream
    ));
    let file = File::create(&path)?;
    Ok((path, file))
}

fn spawn(
    argv: &[String],
    cwd: &PathBuf,
    stdout: File,
    stderr: File,
) -> Result<Child, std::io::Error> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    inherit_safe_environment(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.spawn()
}

fn inherit_safe_environment(command: &mut Command) {
    const SAFE: &[&str] = &[
        "PATH",
        "HOME",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LANG",
        "TERM",
        "SHELL",
        "USER",
        "LOGNAME",
        "SSH_AUTH_SOCK",
    ];
    for key in SAFE {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    for (key, value) in std::env::vars_os() {
        let key_text = key.to_string_lossy();
        if key_text.starts_with("LC_") && !redact::sensitive_env_key(&key_text) {
            command.env(key, value);
        }
    }
}

fn terminate(child: &mut Child) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    unsafe {
        if libc::kill(-(child.id() as i32), libc::SIGTERM) == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        if child.try_wait()?.is_none() {
            let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        child.kill()
    }
}

fn read_tail(path: &PathBuf, limit: usize) -> Result<(String, bool), std::io::Error> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len() as usize;
    let truncated = len > limit;
    if truncated {
        file.seek(SeekFrom::End(-(limit as i64)))?;
    }
    let mut bytes = Vec::with_capacity(len.min(limit));
    file.read_to_end(&mut bytes)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_output_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground(
                &ws,
                &["sh".into(), "-c".into(), "printf hello".into()],
                None,
                Some(2_000),
                false,
            )
            .unwrap();
        assert_eq!(result.stdout_tail, "hello");
        assert!(!result.timed_out);
    }

    #[test]
    fn background_job_can_be_polled() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(
                &ws,
                &["sh".into(), "-c".into(), "printf done".into()],
                None,
                false,
            )
            .unwrap();
        for _ in 0..100 {
            if manager.poll(&started.id).unwrap().state == "exited" {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let (output, _) = manager.output(&started.id, "stdout").unwrap();
        assert_eq!(output, "done");
    }

    #[test]
    fn child_does_not_inherit_sensitive_environment() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let manager = JobManager::new();
        std::env::set_var("WEB_HARNESS_TEST_SECRET_TOKEN", "super-secret-value");
        let result = manager
            .run_foreground(
                &ws,
                &[
                    "sh".into(),
                    "-c".into(),
                    "printf '%s' \"$WEB_HARNESS_TEST_SECRET_TOKEN\"".into(),
                ],
                None,
                Some(2_000),
                false,
            )
            .unwrap();
        std::env::remove_var("WEB_HARNESS_TEST_SECRET_TOKEN");
        assert_eq!(result.stdout_tail, "");
    }

    #[test]
    fn returned_output_is_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground(
                &ws,
                &["sh".into(), "-c".into(), "printf 'API_KEY=abc123'".into()],
                None,
                Some(2_000),
                false,
            )
            .unwrap();
        assert!(!result.stdout_tail.contains("abc123"));
        assert!(result.stdout_tail.contains("[REDACTED]"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_blocks_writes_outside_workspace() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside = outside_dir.path().join("blocked.txt");
        let ws = Workspace::new(workspace_dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground(
                &ws,
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    format!("printf blocked > '{}'", outside.display()),
                ],
                None,
                Some(2_000),
                true,
            )
            .unwrap();
        assert_ne!(result.exit_code, Some(0));
        assert!(!outside.exists());
    }
}
