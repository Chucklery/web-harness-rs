use crate::command_policy;
use crate::env;
use crate::jobs::JobError;
use crate::process;
use crate::sandbox;
use crate::sandbox::NetworkPolicy;
use crate::workspace::Workspace;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub(crate) const OUTPUT_LIMIT: usize = 256 * 1024;
const MAX_STDIN_BYTES: usize = 64 * 1024;

struct OutputState {
    bytes: VecDeque<u8>,
    base_cursor: u64,
    total_bytes: u64,
}

impl OutputState {
    fn new() -> Self {
        Self {
            bytes: VecDeque::with_capacity(OUTPUT_LIMIT),
            base_cursor: 0,
            total_bytes: 0,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        self.bytes.extend(bytes.iter().copied());
        while self.bytes.len() > OUTPUT_LIMIT {
            self.bytes.pop_front();
        }
        self.base_cursor = self.total_bytes.saturating_sub(self.bytes.len() as u64);
    }
}

/// A bounded, cursor-addressable output stream owned by one background job.
///
/// The reader thread always drains the child pipe, but retains at most
/// OUTPUT_LIMIT bytes. This prevents a noisy child from blocking on a full
/// pipe or growing an unbounded temporary file.
pub(crate) struct OutputCapture {
    state: Arc<Mutex<OutputState>>,
    reader: Option<JoinHandle<io::Result<()>>>,
}

impl OutputCapture {
    fn spawn<R: Read + Send + 'static>(stream: R) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(OutputState::new()));
        let writer_state = Arc::clone(&state);
        let reader = thread::Builder::new()
            .name("web-harness-output".into())
            .spawn(move || {
                let mut stream = stream;
                let mut buffer = [0u8; 8192];
                loop {
                    let read = stream.read(&mut buffer)?;
                    if read == 0 {
                        break;
                    }
                    let mut state = writer_state
                        .lock()
                        .map_err(|_| io::Error::other("output buffer lock poisoned"))?;
                    state.append(&buffer[..read]);
                }
                Ok(())
            })?;
        Ok(Self {
            state,
            reader: Some(reader),
        })
    }

    pub(crate) fn finish(&mut self) -> io::Result<()> {
        let Some(reader) = self.reader.take() else {
            return Ok(());
        };
        reader
            .join()
            .map_err(|_| io::Error::other("output reader panicked"))?
    }

    pub(crate) fn snapshot_tail(&self) -> io::Result<(Vec<u8>, bool)> {
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("output buffer lock poisoned"))?;
        Ok((state.bytes.iter().copied().collect(), state.base_cursor > 0))
    }

    pub(crate) fn snapshot_from(
        &self,
        cursor: u64,
        limit: usize,
    ) -> io::Result<(Vec<u8>, u64, bool)> {
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("output buffer lock poisoned"))?;
        let start = cursor.max(state.base_cursor).min(state.total_bytes);
        let offset = start.saturating_sub(state.base_cursor) as usize;
        let available = state.bytes.len().saturating_sub(offset);
        let count = available.min(limit);
        let bytes = state
            .bytes
            .iter()
            .skip(offset)
            .take(count)
            .copied()
            .collect::<Vec<_>>();
        let next_cursor = start.saturating_add(count as u64);
        let truncated = cursor < state.base_cursor || next_cursor < state.total_bytes;
        Ok((bytes, next_cursor, truncated))
    }

    #[cfg(all(test, unix))]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.state.lock().unwrap().bytes.len()
    }
}

pub(crate) struct OutputArtifacts {
    pub(crate) stdout: OutputCapture,
    pub(crate) stderr: OutputCapture,
}

impl OutputArtifacts {
    fn from_streams(stdout: ChildStdout, stderr: ChildStderr) -> io::Result<Self> {
        let stdout = OutputCapture::spawn(stdout)?;
        let stderr = match OutputCapture::spawn(stderr) {
            Ok(stderr) => stderr,
            Err(error) => {
                let mut stdout = stdout;
                let _ = stdout.finish();
                return Err(error);
            }
        };
        Ok(Self { stdout, stderr })
    }

    pub(crate) fn finish(&mut self) -> io::Result<()> {
        self.stdout.finish()?;
        self.stderr.finish()
    }
}

pub(crate) struct SpawnedProcess {
    pub(crate) child: Child,
    pub(crate) artifacts: OutputArtifacts,
    pub(crate) stdin_path: Option<PathBuf>,
}

pub(crate) fn spawn_job(
    workspace: &Workspace,
    argv: &[String],
    cwd: Option<&str>,
    sandboxed: bool,
    network: NetworkPolicy,
    stdin: Option<&[u8]>,
) -> Result<SpawnedProcess, JobError> {
    validate_argv(argv)?;
    if stdin.is_some_and(|input| input.len() > MAX_STDIN_BYTES) {
        return Err(JobError::Invalid(
            "stdin must be at most 65536 bytes".into(),
        ));
    }
    let cwd = match cwd {
        Some(cwd) => workspace.resolve(cwd)?,
        None => workspace.root().to_path_buf(),
    };
    let input = stdin.map(create_stdin_file).transpose()?;
    let (child, artifacts) = match spawn(
        workspace,
        argv,
        &cwd,
        sandboxed,
        network,
        input.as_ref().map(|(path, _)| path.as_path()),
    ) {
        Ok(spawned) => spawned,
        Err(error) => {
            if let Some((path, _)) = input {
                let _ = fs::remove_file(path);
            }
            return Err(error.into());
        }
    };
    Ok(SpawnedProcess {
        child,
        artifacts,
        stdin_path: input.map(|(path, _)| path),
    })
}

fn create_stdin_file(input: &[u8]) -> Result<(PathBuf, File), std::io::Error> {
    static NEXT_STDIN_ID: AtomicU64 = AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "web-harness-{}-stdin-{}.input",
        std::process::id(),
        NEXT_STDIN_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = File::create(&path)?;
    file.write_all(input)?;
    file.seek(SeekFrom::Start(0))?;
    Ok((path, file))
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
    workspace: &Workspace,
    argv: &[String],
    cwd: &Path,
    sandboxed: bool,
    network: NetworkPolicy,
    stdin_path: Option<&Path>,
) -> Result<(Child, OutputArtifacts), std::io::Error> {
    let effective_argv = if sandboxed {
        sandbox::wrap_argv_with_network(workspace, argv, network)
    } else {
        argv.to_vec()
    };
    let mut command = Command::new(&effective_argv[0]);
    command
        .args(&effective_argv[1..])
        .current_dir(cwd)
        .stdin(match stdin_path {
            Some(path) => Stdio::from(File::open(path)?),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    env::apply(&mut command);
    if sandboxed {
        command.env(sandbox::SANDBOX_ENV_MARKER, "seatbelt");
    }
    let mut child = process::spawn_in_own_group(&mut command)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child stdout pipe unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("child stderr pipe unavailable"))?;
    let artifacts = match OutputArtifacts::from_streams(stdout, stderr) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    Ok((child, artifacts))
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
