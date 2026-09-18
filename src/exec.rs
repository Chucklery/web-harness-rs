use crate::jobs::{ExecResult, JobError, JobManager, JobStatus};
use crate::workspace::Workspace;

pub fn foreground(
    manager: &JobManager,
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    timeout_ms: Option<u64>,
    sandboxed: bool,
) -> Result<ExecResult, JobError> {
    manager.run_foreground(workspace, argv, cwd, timeout_ms, sandboxed)
}

pub fn background(
    manager: &mut JobManager,
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    sandboxed: bool,
) -> Result<JobStatus, JobError> {
    manager.start(workspace, argv, cwd, sandboxed)
}
