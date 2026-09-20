use crate::command_policy;
use crate::env;
use crate::process;
use crate::redact;
use crate::sandbox;
use crate::workspace::{Workspace, WorkspaceError};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
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

impl Job {
    /// Marks the job as finished once the child has been reaped.
    ///
    /// A process terminated by a signal has no exit code; jobs report `-1` for
    /// that case instead of exposing the raw `None`. This is also the only
    /// transition out of the running state, so `is_running` stays the single
    /// definition of "live".
    fn record_exit(&mut self, status: std::process::ExitStatus) {
        self.exit_code = status.code().or(Some(-1));
    }

    fn is_running(&self) -> bool {
        self.exit_code.is_none()
    }

    fn remove_artifacts(&self) {
        let _ = fs::remove_file(&self.stdout_path);
        let _ = fs::remove_file(&self.stderr_path);
    }
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
        let spawned = spawn_job(workspace, argv, cwd, sandboxed)?;
        let SpawnedProcess {
            mut child,
            artifacts,
        } = spawned;
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

        let (stdout_tail, stdout_truncated) = read_tail(&artifacts.stdout.path, OUTPUT_TAIL_BYTES)?;
        let (stderr_tail, stderr_truncated) = read_tail(&artifacts.stderr.path, OUTPUT_TAIL_BYTES)?;
        artifacts.remove();
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
        self.refresh();
        if self.jobs.values().filter(|job| job.is_running()).count() >= MAX_BACKGROUND_JOBS {
            return Err(JobError::Limit);
        }
        let spawned = spawn_job(workspace, argv, cwd, sandboxed)?;
        let id = format!(
            "job_{}_{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        self.jobs.insert(
            id.clone(),
            Job {
                child: spawned.child,
                stdout_path: spawned.artifacts.stdout.path,
                stderr_path: spawned.artifacts.stderr.path,
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
            let was_running = job.is_running();
            if was_running {
                if let Some(status) = job.child.try_wait()? {
                    job.record_exit(status);
                }
            }
            (
                if job.is_running() {
                    "running".to_string()
                } else {
                    "exited".to_string()
                },
                job.exit_code,
                was_running,
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
            let was_running = job.is_running();
            if was_running {
                terminate(&mut job.child)?;
                let status = job.child.wait()?;
                job.record_exit(status);
            }
            (job.exit_code, was_running)
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
            if job.is_running() {
                if let Ok(Some(status)) = job.child.try_wait() {
                    job.record_exit(status);
                    completed.push(id.clone());
                }
            }
        }
        for id in completed {
            self.record_completed(&id);
        }
    }

    /// Records a finished job and evicts the oldest completed jobs once the
    /// retention limit is exceeded. Running jobs are never evicted.
    fn record_completed(&mut self, id: &str) {
        self.completed_order.push_back(id.to_string());
        while self.completed_order.len() > MAX_RETAINED_COMPLETED_JOBS {
            let Some(evicted_id) = self.completed_order.pop_front() else {
                break;
            };
            if let Some(job) = self.jobs.remove(&evicted_id) {
                job.remove_artifacts();
            }
        }
    }
}

impl Drop for JobManager {
    fn drop(&mut self) {
        for job in self.jobs.values_mut() {
            if job.is_running() {
                let _ = terminate(&mut job.child);
                let _ = job.child.wait();
            }
            job.remove_artifacts();
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

/// One captured output stream: the file the child writes to and its path, which
/// is kept so the artifact can be read back and cleaned up later.
struct OutputArtifact {
    path: PathBuf,
    file: File,
}

impl OutputArtifact {
    fn create(stream: &str) -> Result<Self, std::io::Error> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "web-harness-{}-{}-{}.log",
            std::process::id(),
            id,
            stream
        ));
        let file = File::create(&path)?;
        Ok(Self { path, file })
    }
}

/// Both captured output streams of a child process.
struct OutputArtifacts {
    stdout: OutputArtifact,
    stderr: OutputArtifact,
}

impl OutputArtifacts {
    fn create() -> Result<Self, std::io::Error> {
        Ok(Self {
            stdout: OutputArtifact::create("stdout")?,
            stderr: OutputArtifact::create("stderr")?,
        })
    }

    fn remove(&self) {
        let _ = fs::remove_file(&self.stdout.path);
        let _ = fs::remove_file(&self.stderr.path);
    }
}

struct SpawnedProcess {
    child: Child,
    artifacts: OutputArtifacts,
}

/// Validates the request, starts the child in its own process group, and returns
/// the running child together with the artifacts capturing its output.
fn spawn_job(
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    sandboxed: bool,
) -> Result<SpawnedProcess, JobError> {
    validate_argv(argv)?;
    let cwd = resolve_cwd(workspace, cwd)?;
    let artifacts = OutputArtifacts::create()?;
    let effective_argv = if sandboxed {
        sandbox::wrap_argv(workspace, argv)
    } else {
        argv.to_vec()
    };
    let child = spawn(&effective_argv, &cwd, &artifacts, sandboxed)?;
    Ok(SpawnedProcess { child, artifacts })
}

fn spawn(
    argv: &[String],
    cwd: &Path,
    artifacts: &OutputArtifacts,
    sandboxed: bool,
) -> Result<Child, std::io::Error> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(artifacts.stdout.file.try_clone()?)
        .stderr(artifacts.stderr.file.try_clone()?);
    env::apply(&mut command);
    if sandboxed {
        command.env(sandbox::SANDBOX_ENV_MARKER, "seatbelt");
    }
    // A missing libc call can only fail before `exec`, and the resulting spawn
    // error is reported like any other launch failure.
    let _ = process::detach_into_own_group(&mut command);
    command.spawn()
}

fn terminate(child: &mut Child) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        // Reap the child if the process group was already gone; signalling an
        // absent group is expected during cleanup and is not an error.
        if let Err(error) = process::terminate_process_group(child.id()) {
            if child.try_wait()?.is_none() {
                return Err(std::io::Error::other(error.to_string()));
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        child.kill()
    }
}

fn read_tail(path: &Path, limit: usize) -> Result<(String, bool), std::io::Error> {
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
                &["printf".into(), "hello".into()],
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
            .run_foreground(&ws, &["pwd".into()], Some("../"), Some(2_000), false)
            .unwrap_err();
        assert!(matches!(
            error,
            JobError::Workspace(WorkspaceError::OutsideWorkspace(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn inline_evaluation_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let manager = JobManager::new();
        let error = manager
            .run_foreground(
                &ws,
                &["sh".into(), "-c".into(), "printf hello".into()],
                None,
                Some(2_000),
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("inline evaluation"));
    }

    #[cfg(unix)]
    #[test]
    fn background_job_can_be_polled() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["printf".into(), "done".into()], None, false)
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
                .start(&ws, &["printf".into(), "done".into()], None, false)
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
        let script = dir.path().join("dump_env.sh");
        fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' \"$WEB_HARNESS_TEST_SECRET_TOKEN\"\n",
        )
        .unwrap();
        std::env::set_var("WEB_HARNESS_TEST_SECRET_TOKEN", "super-secret-value");
        let result = manager
            .run_foreground(
                &ws,
                &["sh".into(), "dump_env.sh".into()],
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
        fs::write(dir.path().join("secret.txt"), "API_KEY=abc123").unwrap();
        let result = manager
            .run_foreground(
                &ws,
                &["cat".into(), "secret.txt".into()],
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
        let script = workspace_dir.path().join("dev_null.sh");
        fs::write(&script, "#!/bin/sh\nprintf ok >/dev/null\n").unwrap();
        let result = manager
            .run_foreground(
                &ws,
                &["/bin/sh".into(), "dev_null.sh".into()],
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
        let script = workspace_dir.path().join("blocked_write.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\nprintf blocked > '{}'\n", outside.display()),
        )
        .unwrap();
        let result = manager
            .run_foreground(
                &ws,
                &["/bin/sh".into(), "blocked_write.sh".into()],
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
        let script = workspace_dir.path().join("allowed_writes.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf workspace > allowed.txt && printf temp > '{}'\n",
                temp_file.display()
            ),
        )
        .unwrap();
        let result = manager
            .run_foreground(
                &ws,
                &["/bin/sh".into(), "allowed_writes.sh".into()],
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
