use crate::jobs::{ExecResult, JobError, JobManager, JobStatus};
use crate::sandbox::NetworkPolicy;
use crate::workspace::Workspace;

pub fn foreground(
    manager: &JobManager,
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    timeout_ms: Option<u64>,
    sandboxed: bool,
    network: NetworkPolicy,
    stdin: Option<&[u8]>,
) -> Result<ExecResult, JobError> {
    manager.run_foreground_with_network(workspace, argv, cwd, timeout_ms, sandboxed, network, stdin)
}

pub fn background(
    manager: &mut JobManager,
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    sandboxed: bool,
    network: NetworkPolicy,
    stdin: Option<&[u8]>,
) -> Result<JobStatus, JobError> {
    manager.start_with_network(workspace, argv, cwd, sandboxed, network, stdin)
}
