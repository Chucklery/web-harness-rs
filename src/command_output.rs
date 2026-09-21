use std::io::{self, Read};
use std::process::{Child, ExitStatus};
use std::thread;

type StreamReader = thread::JoinHandle<io::Result<(Vec<u8>, bool)>>;

pub struct BoundedChildOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr: Vec<u8>,
    pub stderr_truncated: bool,
}

pub fn collect(
    mut child: Child,
    stdout_limit: usize,
    stderr_limit: usize,
) -> io::Result<BoundedChildOutput> {
    let stdout = child
        .stdout
        .take()
        .map(|stream| thread::spawn(move || read_bounded(stream, stdout_limit)));
    let stderr = child
        .stderr
        .take()
        .map(|stream| thread::spawn(move || read_bounded(stream, stderr_limit)));
    let status = child.wait()?;
    let (stdout, stdout_truncated) = join_stream(stdout)?;
    let (stderr, stderr_truncated) = join_stream(stderr)?;
    Ok(BoundedChildOutput {
        status,
        stdout,
        stdout_truncated,
        stderr,
        stderr_truncated,
    })
}

fn read_bounded(mut stream: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(output.len());
        let keep = remaining.min(read);
        output.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok((output, truncated))
}

fn read_bounded_tail(mut stream: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..read]);
        if output.len() > limit {
            let remove = output.len() - limit;
            output.drain(..remove);
            truncated = true;
        }
    }
    Ok((output, truncated))
}

pub fn read_tail_stream(
    stream: impl Read + Send + 'static,
    limit: usize,
) -> io::Result<(Vec<u8>, bool)> {
    read_bounded_tail(stream, limit)
}

fn join_stream(stream: Option<StreamReader>) -> io::Result<(Vec<u8>, bool)> {
    match stream {
        Some(stream) => stream
            .join()
            .map_err(|_| io::Error::other("bounded output reader panicked"))?,
        None => Ok((Vec::new(), false)),
    }
}
