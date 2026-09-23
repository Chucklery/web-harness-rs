use crate::job_process::{spawn_job, terminate, OutputArtifacts, SpawnedProcess, OUTPUT_LIMIT};
use crate::redact;
use crate::sandbox::NetworkPolicy;
use crate::workspace::{Workspace, WorkspaceError};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_BACKGROUND_JOBS: usize = 2;
const MAX_RETAINED_COMPLETED_JOBS: usize = 4;
const OUTPUT_TAIL_BYTES: usize = OUTPUT_LIMIT;
const MAX_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_WAIT: Duration = Duration::from_secs(60);

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

#[derive(Debug, Serialize)]
pub struct JobOutputChunk {
    pub text: String,
    pub next_cursor: u64,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct JobWaitResult {
    pub status: JobStatus,
    pub stdout: JobOutputChunk,
    pub stderr: JobOutputChunk,
}

struct Job {
    child: Child,
    artifacts: OutputArtifacts,
    stdin_path: Option<PathBuf>,
    state: JobState,
}

#[derive(Clone, Copy)]
enum JobState {
    Running,
    Exited(i32),
    Cancelled(i32),
}

#[derive(Clone, Copy)]
enum JobTransition {
    Observe,
    Cancel,
}

impl Job {
    /// Applies the only state transition out of `Running`.
    ///
    /// The return value is true exactly once: when this call observes or
    /// causes process completion. Callers use it to retain the job once.
    fn transition(&mut self, transition: JobTransition) -> Result<bool, std::io::Error> {
        if !self.is_running() {
            return Ok(false);
        }

        let (status, cancelled) = match transition {
            JobTransition::Observe => (self.child.try_wait()?, false),
            JobTransition::Cancel => match self.child.try_wait()? {
                Some(status) => (Some(status), false),
                None => {
                    terminate(&mut self.child)?;
                    (Some(self.child.wait()?), true)
                }
            },
        };
        let Some(status) = status else {
            return Ok(false);
        };
        // A shell can exit while a descendant still holds the inherited
        // stdout/stderr pipe. Terminate the owned process group before joining
        // output readers so completion cannot wait on an orphaned descendant.
        if matches!(transition, JobTransition::Observe) {
            terminate(&mut self.child)?;
        }
        let exit_code = status.code().unwrap_or(-1);
        self.state = if cancelled {
            JobState::Cancelled(exit_code)
        } else {
            JobState::Exited(exit_code)
        };
        Ok(true)
    }

    fn is_running(&self) -> bool {
        matches!(self.state, JobState::Running)
    }

    fn status(&self, id: &str) -> JobStatus {
        let (state, exit_code) = match self.state {
            JobState::Running => ("running", None),
            JobState::Exited(exit_code) => ("exited", Some(exit_code)),
            JobState::Cancelled(exit_code) => ("cancelled", Some(exit_code)),
        };
        JobStatus {
            id: id.to_string(),
            state: state.into(),
            exit_code,
        }
    }

    fn finish_output(&mut self) -> Result<(), std::io::Error> {
        self.artifacts.finish()
    }

    fn remove_artifacts(&mut self) {
        let _ = self.finish_output();
        if let Some(path) = &self.stdin_path {
            let _ = fs::remove_file(path);
        }
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

    #[cfg(any(test, feature = "release-tools"))]
    pub fn run_foreground(
        &self,
        workspace: &Workspace,
        argv: &[String],
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        sandboxed: bool,
    ) -> Result<ExecResult, JobError> {
        self.run_foreground_with_network(
            workspace,
            argv,
            cwd,
            timeout_ms,
            sandboxed,
            NetworkPolicy::Deny,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_foreground_with_network(
        &self,
        workspace: &Workspace,
        argv: &[String],
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        sandboxed: bool,
        network: NetworkPolicy,
        stdin: Option<&[u8]>,
    ) -> Result<ExecResult, JobError> {
        let spawned = spawn_job(workspace, argv, cwd, sandboxed, network, stdin)?;
        let SpawnedProcess {
            mut child,
            mut artifacts,
            stdin_path,
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

        terminate(&mut child)?;
        artifacts.finish()?;
        if let Some(path) = stdin_path {
            let _ = fs::remove_file(path);
        }
        let (stdout, stdout_truncated) = artifacts.stdout.snapshot_tail()?;
        let (stderr, stderr_truncated) = artifacts.stderr.snapshot_tail()?;
        Ok(ExecResult {
            exit_code,
            stdout_tail: redact::text(&String::from_utf8_lossy(&stdout)),
            stderr_tail: redact::text(&String::from_utf8_lossy(&stderr)),
            stdout_truncated,
            stderr_truncated,
            timed_out,
        })
    }

    #[cfg(all(test, unix))]
    pub fn start(
        &mut self,
        workspace: &Workspace,
        argv: &[String],
        cwd: Option<&str>,
        sandboxed: bool,
    ) -> Result<JobStatus, JobError> {
        self.start_with_network(workspace, argv, cwd, sandboxed, NetworkPolicy::Deny, None)
    }

    pub fn start_with_network(
        &mut self,
        workspace: &Workspace,
        argv: &[String],
        cwd: Option<&str>,
        sandboxed: bool,
        network: NetworkPolicy,
        stdin: Option<&[u8]>,
    ) -> Result<JobStatus, JobError> {
        self.refresh();
        if self.jobs.values().filter(|job| job.is_running()).count() >= MAX_BACKGROUND_JOBS {
            return Err(JobError::Limit);
        }
        let spawned = spawn_job(workspace, argv, cwd, sandboxed, network, stdin)?;
        let id = format!(
            "job_{}_{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        self.jobs.insert(
            id.clone(),
            Job {
                child: spawned.child,
                artifacts: spawned.artifacts,
                stdin_path: spawned.stdin_path,
                state: JobState::Running,
            },
        );
        Ok(JobStatus {
            id,
            state: "running".into(),
            exit_code: None,
        })
    }

    pub fn poll(&mut self, id: &str) -> Result<JobStatus, JobError> {
        self.transition_job(id, JobTransition::Observe)
    }

    pub fn cancel(&mut self, id: &str) -> Result<JobStatus, JobError> {
        self.transition_job(id, JobTransition::Cancel)
    }

    pub fn output(&self, id: &str, stream: &str) -> Result<(String, bool), JobError> {
        let job = self
            .jobs
            .get(id)
            .ok_or_else(|| JobError::NotFound(id.to_string()))?;
        let capture = match stream {
            "stdout" => &job.artifacts.stdout,
            "stderr" => &job.artifacts.stderr,
            _ => return Err(JobError::Invalid("stream must be stdout or stderr".into())),
        };
        let (bytes, truncated) = capture.snapshot_tail()?;
        Ok((redact::text(&String::from_utf8_lossy(&bytes)), truncated))
    }

    pub fn output_from(
        &self,
        id: &str,
        stream: &str,
        cursor: u64,
    ) -> Result<JobOutputChunk, JobError> {
        let job = self
            .jobs
            .get(id)
            .ok_or_else(|| JobError::NotFound(id.to_string()))?;
        let capture = match stream {
            "stdout" => &job.artifacts.stdout,
            "stderr" => &job.artifacts.stderr,
            _ => return Err(JobError::Invalid("stream must be stdout or stderr".into())),
        };
        let (bytes, next_cursor, truncated) = capture.snapshot_from(cursor, OUTPUT_TAIL_BYTES)?;
        Ok(JobOutputChunk {
            text: redact::text(&String::from_utf8_lossy(&bytes)),
            next_cursor,
            truncated,
        })
    }

    pub fn wait(&mut self, id: &str, timeout_ms: Option<u64>) -> Result<JobStatus, JobError> {
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(10_000)).min(MAX_WAIT);
        let start = Instant::now();
        loop {
            let status = self.poll(id)?;
            if status.state != "running" || start.elapsed() >= timeout {
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn wait_with_output(
        &mut self,
        id: &str,
        timeout_ms: Option<u64>,
        stdout_cursor: u64,
        stderr_cursor: u64,
    ) -> Result<JobWaitResult, JobError> {
        let status = self.wait(id, timeout_ms)?;
        let stdout = self.output_from(id, "stdout", stdout_cursor)?;
        let stderr = self.output_from(id, "stderr", stderr_cursor)?;
        Ok(JobWaitResult {
            status,
            stdout,
            stderr,
        })
    }

    pub fn list(&mut self) -> Vec<JobStatus> {
        self.refresh();
        let mut statuses = self
            .jobs
            .iter()
            .map(|(id, job)| job.status(id))
            .collect::<Vec<_>>();
        statuses.sort_by(|left, right| left.id.cmp(&right.id));
        statuses
    }

    fn refresh(&mut self) {
        let ids: Vec<_> = self.jobs.keys().cloned().collect();
        for id in ids {
            let _ = self.transition_job(&id, JobTransition::Observe);
        }
    }

    fn transition_job(
        &mut self,
        id: &str,
        transition: JobTransition,
    ) -> Result<JobStatus, JobError> {
        let completed = {
            let job = self
                .jobs
                .get_mut(id)
                .ok_or_else(|| JobError::NotFound(id.to_string()))?;
            job.transition(transition)?
        };
        if completed {
            self.jobs
                .get_mut(id)
                .expect("transitioned job is retained")
                .finish_output()?;
            self.record_completed(id);
        }
        Ok(self
            .jobs
            .get(id)
            .expect("transitioned job is retained")
            .status(id))
    }

    /// Records a finished job and evicts the oldest completed jobs once the
    /// retention limit is exceeded. Running jobs are never evicted.
    fn record_completed(&mut self, id: &str) {
        self.completed_order.push_back(id.to_string());
        while self.completed_order.len() > MAX_RETAINED_COMPLETED_JOBS {
            let Some(evicted_id) = self.completed_order.pop_front() else {
                break;
            };
            if let Some(mut job) = self.jobs.remove(&evicted_id) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use crate::sandbox;

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
    fn foreground_accepts_bounded_one_shot_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let manager = JobManager::new();
        let result = manager
            .run_foreground_with_network(
                &ws,
                &["cat".into()],
                None,
                Some(2_000),
                false,
                NetworkPolicy::Deny,
                Some(b"hello from stdin"),
            )
            .unwrap();
        assert_eq!(result.stdout_tail, "hello from stdin");
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
    fn wait_and_output_cursor_observe_one_job() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["printf".into(), "abcdef".into()], None, false)
            .unwrap();
        assert_eq!(
            manager.wait(&started.id, Some(2_000)).unwrap().state,
            "exited"
        );
        let chunk = manager.output_from(&started.id, "stdout", 0).unwrap();
        assert_eq!(chunk.text, "abcdef");
        assert_eq!(chunk.next_cursor, 6);
        assert!(!chunk.truncated);
        assert_eq!(manager.list().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn wait_returns_incremental_stdout_and_stderr_together() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        fs::write(
            dir.path().join("emit.sh"),
            "#!/bin/sh\nprintf 'out-one'\nprintf 'err-one' >&2\n",
        )
        .unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["sh".into(), "emit.sh".into()], None, false)
            .unwrap();

        let first = manager
            .wait_with_output(&started.id, Some(2_000), 0, 0)
            .unwrap();
        assert_eq!(first.status.state, "exited");
        assert_eq!(first.stdout.text, "out-one");
        assert_eq!(first.stderr.text, "err-one");
        assert_eq!(first.stdout.next_cursor, 7);
        assert_eq!(first.stderr.next_cursor, 7);
        assert!(!first.stdout.truncated);
        assert!(!first.stderr.truncated);

        let second = manager
            .wait_with_output(
                &started.id,
                Some(1),
                first.stdout.next_cursor,
                first.stderr.next_cursor,
            )
            .unwrap();
        assert_eq!(second.stdout.text, "");
        assert_eq!(second.stderr.text, "");
    }

    #[cfg(unix)]
    #[test]
    fn background_output_is_bounded_and_reports_cursor_loss() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["seq".into(), "100000".into()], None, false)
            .unwrap();
        assert_eq!(
            manager.wait(&started.id, Some(5_000)).unwrap().state,
            "exited"
        );
        let job = manager.jobs.get(&started.id).unwrap();
        assert!(job.artifacts.stdout.retained_bytes() <= OUTPUT_LIMIT);
        let chunk = manager.output_from(&started.id, "stdout", 0).unwrap();
        assert!(chunk.truncated);
        assert!(chunk.next_cursor > 0);
    }

    #[cfg(unix)]
    #[test]
    fn completion_cleans_descendants_that_hold_output_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        fs::write(
            dir.path().join("descendant.sh"),
            "#!/bin/sh\nsleep 10 &\nexit 0\n",
        )
        .unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["sh".into(), "descendant.sh".into()], None, false)
            .unwrap();
        let began = Instant::now();
        assert_eq!(
            manager.wait(&started.id, Some(2_000)).unwrap().state,
            "exited"
        );
        assert!(began.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn running_job_is_not_recorded_as_completed() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["/bin/sleep".into(), "2".into()], None, false)
            .unwrap();

        assert_eq!(manager.poll(&started.id).unwrap().state, "running");
        assert!(manager.completed_order.is_empty());
        assert!(manager.jobs.contains_key(&started.id));
    }

    #[cfg(unix)]
    #[test]
    fn repeated_poll_records_completion_once() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["printf".into(), "done".into()], None, false)
            .unwrap();

        loop {
            if manager.poll(&started.id).unwrap().state == "exited" {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(manager.poll(&started.id).unwrap().state, "exited");
        assert_eq!(
            manager
                .completed_order
                .iter()
                .filter(|id| *id == &started.id)
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancel_after_poll_records_completion_once() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let started = manager
            .start(&ws, &["/bin/sleep".into(), "2".into()], None, false)
            .unwrap();

        assert_eq!(manager.poll(&started.id).unwrap().state, "running");
        assert_eq!(manager.cancel(&started.id).unwrap().state, "cancelled");
        assert_eq!(manager.poll(&started.id).unwrap().state, "cancelled");
        assert_eq!(manager.completed_order.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn completed_background_jobs_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let mut manager = JobManager::new();
        let mut ids = Vec::new();
        for _ in 0..(MAX_RETAINED_COMPLETED_JOBS + 2) {
            let started = manager
                .start(&ws, &["printf".into(), "done".into()], None, false)
                .unwrap();
            let id = started.id.clone();
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
        assert!(manager
            .jobs
            .values()
            .all(|job| job.artifacts.stdout.retained_bytes() <= OUTPUT_LIMIT));
        assert!(manager
            .jobs
            .values()
            .all(|job| job.artifacts.stderr.retained_bytes() <= OUTPUT_LIMIT));
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
        let wrapped = sandbox::wrap_argv_with_state(&ws, &argv, true, NetworkPolicy::Deny);
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

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_outbound_approval_allows_an_outgoing_connection() {
        use std::net::TcpListener;

        if !crate::sandbox::can_upgrade_network() {
            eprintln!("skipping: outbound upgrade is unavailable inside the current sandbox");
            return;
        }
        if !std::path::Path::new("/usr/bin/nc").exists() {
            eprintln!("skipping: /usr/bin/nc is unavailable");
            return;
        }
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
            .run_foreground_with_network(
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
                NetworkPolicy::Outbound,
                None,
            )
            .unwrap();
        assert_eq!(result.exit_code, Some(0), "{}", result.stderr_tail);
        assert!(listener.accept().is_ok());
    }
}
