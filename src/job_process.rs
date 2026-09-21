use crate::command_policy;
use crate::env;
use crate::jobs::JobError;
use crate::process;
use crate::sandbox;
use crate::workspace::Workspace;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ARTIFACT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct OutputArtifact {
    pub(crate) path: PathBuf,
    file: File,
}

impl OutputArtifact {
    fn create(stream: &str) -> Result<Self, std::io::Error> {
        let id = NEXT_ARTIFACT_ID.fetch_add(1, Ordering::Relaxed);
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

pub(crate) struct OutputArtifacts {
    pub(crate) stdout: OutputArtifact,
    pub(crate) stderr: OutputArtifact,
}

impl OutputArtifacts {
    fn create() -> Result<Self, std::io::Error> {
        Ok(Self {
            stdout: OutputArtifact::create("stdout")?,
            stderr: OutputArtifact::create("stderr")?,
        })
    }

    pub(crate) fn remove(&self) {
        let _ = fs::remove_file(&self.stdout.path);
        let _ = fs::remove_file(&self.stderr.path);
    }
}

pub(crate) struct SpawnedProcess {
    pub(crate) child: Child,
    pub(crate) artifacts: OutputArtifacts,
}

pub(crate) fn spawn_job(
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    sandboxed: bool,
) -> Result<SpawnedProcess, JobError> {
    validate_argv(argv)?;
    let cwd = match cwd {
        Some(cwd) => workspace.resolve(cwd)?,
        None => workspace.root().to_path_buf(),
    };
    let artifacts = OutputArtifacts::create()?;
    let effective_argv = if sandboxed {
        sandbox::wrap_argv(workspace, argv)
    } else {
        argv.to_vec()
    };
    let child = spawn(&effective_argv, &cwd, &artifacts, sandboxed)?;
    Ok(SpawnedProcess { child, artifacts })
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
    let _ = process::detach_into_own_group(&mut command);
    command.spawn()
}

pub(crate) fn terminate(child: &mut Child) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
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

pub(crate) fn read_tail(path: &Path, limit: usize) -> Result<(String, bool), std::io::Error> {
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
