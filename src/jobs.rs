use crate::command_policy;
use crate::redact;
use crate::sandbox;
use crate::workspace::{Workspace, WorkspaceError};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_BACKGROUND_JOBS: usize = 2;
const MAX_RETAINED_COMPLETED_JOBS: usize = 4;
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
    completed_order: VecDeque<String>,
}

impl JobManager {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            completed_order: VecDeque::new(),
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
        let mut child = spawn(&effective_argv, &cwd, stdout, stderr, sandboxed)?;
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
        let child = spawn(&effective_argv, &cwd, stdout, stderr, sandboxed)?;
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
        let (state, exit_code, newly_completed) = {
            let job = self
                .jobs
                .get_mut(id)
                .ok_or_else(|| JobError::NotFound(id.to_string()))?;
            let was_running = job.exit_code.is_none();
            if was_running {
                if let Some(status) = job.child.try_wait()? {
                    job.exit_code = status.code().or(Some(-1));
                }
            }
            (
                if job.exit_code.is_some() {
                    "exited".to_string()
                } else {
                    "running".to_string()
                },
                job.exit_code,
                was_running && job.exit_code.is_some(),
            )
        };
        if newly_completed {
            self.record_completed(id);
        }
        Ok(JobStatus {
            id: id.to_string(),
            state,
            exit_code,
        })
    }

    pub fn cancel(&mut self, id: &str) -> Result<JobStatus, JobError> {
        let (exit_code, newly_completed) = {
            let job = self
                .jobs
                .get_mut(id)
                .ok_or_else(|| JobError::NotFound(id.to_string()))?;
            let was_running = job.exit_code.is_none();
            if was_running {
                terminate(&mut job.child)?;
                let status = job.child.wait()?;
                job.exit_code = status.code().or(Some(-1));
            }
            (job.exit_code, was_running && job.exit_code.is_some())
        };
        if newly_completed {
            self.record_completed(id);
        }
        Ok(JobStatus {
            id: id.to_string(),
            state: "cancelled".into(),
            exit_code,
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
        let mut completed = Vec::new();
        for (id, job) in &mut self.jobs {
            if job.exit_code.is_none() {
                if let Ok(Some(status)) = job.child.try_wait() {
                    job.exit_code = status.code().or(Some(-1));
                    completed.push(id.clone());
                }
            }
        }
        for id in completed {
            self.record_completed(&id);
        }
    }

    fn record_completed(&mut self, id: &str) {
        self.completed_order.push_back(id.to_string());
        while self.completed_order.len() > MAX_RETAINED_COMPLETED_JOBS {
            if let Some(evicted_id) = self.completed_order.pop_front() {
                if let Some(job) = self.jobs.remove(&evicted_id) {
                    let _ = fs::remove_file(job.stdout_path);
                    let _ = fs::remove_file(job.stderr_path);
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
    command_policy::validate_argv(argv).map_err(|error| JobError::Invalid(error.to_string()))?;
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
    sandboxed: bool,
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
    if sandboxed {
        command.env(sandbox::SANDBOX_ENV_MARKER, "seatbelt");
    }
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

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn execution_rejects_parent_cwd_escape() {
        let outer = tempfile::tempdir().unwrap();
        let workspace_dir = outer.path().join("workspace");
        fs::create_dir(&workspace_dir).unwrap();
        let ws = Workspace::new(&workspace_dir).unwrap();
        let manager = JobManager::new();
        let error = manager
            .run_foreground(
                &ws,
                &["sh".into(), "-c".into(), "pwd".into()],
                Some("../"),
                Some(2_000),
                false,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            JobError::Workspace(WorkspaceError::OutsideWorkspace(_))
        ));
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn completed_background_jobs_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let mut ids = Vec::new();
        let mut first_artifacts = None;

        for index in 0..(MAX_RETAINED_COMPLETED_JOBS + 2) {
            let started = manager
                .start(
                    &ws,
                    &["sh".into(), "-c".into(), "printf done".into()],
                    None,
                    false,
                )
                .unwrap();
            let id = started.id.clone();
            if index == 0 {
                let job = manager.jobs.get(&id).unwrap();
                first_artifacts = Some((job.stdout_path.clone(), job.stderr_path.clone()));
            }
            loop {
                let status = manager.poll(&id).unwrap();
                if status.state == "exited" {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            ids.push(id);
        }

        assert!(manager.jobs.len() <= MAX_RETAINED_COMPLETED_JOBS);
        assert!(!manager.jobs.contains_key(&ids[0]));
        assert!(!manager.jobs.contains_key(&ids[1]));
        let (stdout_path, stderr_path) = first_artifacts.unwrap();
        assert!(!stdout_path.exists());
        assert!(!stderr_path.exists());
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[test]
    fn command_policy_rejects_host_control_execution() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let manager = JobManager::new();
        let error = manager
            .run_foreground(
                &ws,
                &["sudo".into(), "true".into()],
                None,
                Some(2_000),
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("host-control executable"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_allows_dev_null() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(workspace_dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground(
                &ws,
                &["/bin/sh".into(), "-c".into(), "printf ok >/dev/null".into()],
                None,
                Some(2_000),
                true,
            )
            .unwrap();
        assert_eq!(result.exit_code, Some(0), "{}", result.stderr_tail);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_allows_git_to_open_dev_null() {
        if !std::path::Path::new("/usr/bin/git").exists() {
            return;
        }
        let workspace_dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(workspace_dir.path()).unwrap();
        let manager = JobManager::new();
        // `git init` is one of the shortest developer commands that fails
        // outright when /dev/null is denied under Seatbelt.
        let init = manager
            .run_foreground(
                &ws,
                &["/usr/bin/git".into(), "init".into()],
                None,
                Some(5_000),
                true,
            )
            .unwrap();
        assert_eq!(init.exit_code, Some(0), "{}", init.stderr_tail);
        let status = manager
            .run_foreground(
                &ws,
                &["/usr/bin/git".into(), "status".into(), "--short".into()],
                None,
                Some(5_000),
                true,
            )
            .unwrap();
        assert_eq!(status.exit_code, Some(0), "{}", status.stderr_tail);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn nested_web_harness_sandbox_does_not_reapply_seatbelt() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let argv = vec!["/bin/echo".to_string(), "ok".to_string()];
        let wrapped = sandbox::wrap_argv_with_state(&ws, &argv, true);
        assert_eq!(wrapped, argv);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_blocks_writes_outside_workspace() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let Some(home) = std::env::var_os("HOME") else {
            eprintln!("skipping: HOME is not set");
            return;
        };
        // Setting up the out-of-workspace target is a *host* operation. When
        // this test suite itself runs inside a web-harness Seatbelt profile the
        // host denies it, which is a property of the harness, not of the code
        // under test. Skip rather than fail so the suite stays usable from
        // `web-harness exec cargo test`.
        let Ok(outside_dir) = tempfile::Builder::new()
            .prefix("web-harness-outside-")
            .tempdir_in(home)
        else {
            eprintln!("skipping: host denied creating the out-of-workspace directory");
            return;
        };
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

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_allows_workspace_and_temp_writes() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(workspace_dir.path()).unwrap();
        let manager = JobManager::new();
        let temp_file =
            std::env::temp_dir().join(format!("web-harness-sandbox-test-{}", std::process::id()));
        let result = manager
            .run_foreground(
                &ws,
                &[
                    "/bin/sh".into(),
                    "-c".into(),
                    format!(
                        "printf workspace > allowed.txt && printf temp > '{}'",
                        temp_file.display()
                    ),
                ],
                None,
                Some(2_000),
                true,
            )
            .unwrap();
        assert_eq!(result.exit_code, Some(0), "{}", result.stderr_tail);
        assert!(workspace_dir.path().join("allowed.txt").exists());
        assert!(temp_file.exists());
        let _ = fs::remove_file(temp_file);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_allows_reading_system_tools() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(workspace_dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground(
                &ws,
                &["/bin/cat".into(), "/etc/hosts".into()],
                None,
                Some(2_000),
                true,
            )
            .unwrap();
        assert_eq!(result.exit_code, Some(0), "{}", result.stderr_tail);
        assert!(!result.stdout_tail.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_blocks_local_network_connections() {
        use std::net::TcpListener;

        if !std::path::Path::new("/usr/bin/nc").exists() {
            return;
        }
        // Binding the listener is host work; an enclosing web-harness sandbox
        // denies it. Skip instead of failing, since that is the harness working
        // as intended rather than a regression in the sandbox code.
        let Ok(listener) = TcpListener::bind("127.0.0.1:0") else {
            eprintln!("skipping: host denied binding a loopback listener");
            return;
        };
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let workspace_dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(workspace_dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground(
                &ws,
                &[
                    "/usr/bin/nc".into(),
                    "-z".into(),
                    "127.0.0.1".into(),
                    port.to_string(),
                ],
                None,
                Some(2_000),
                true,
            )
            .unwrap();
        assert_ne!(result.exit_code, Some(0));
        assert!(listener.accept().is_err());
    }
}
