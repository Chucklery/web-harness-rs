//! Shared child-process primitives.
//!
//! web-harness never leaves a process it spawned behind: every child is started
//! in its own process group and terminated as a group. These helpers are the
//! single place that encodes those two rules, so the tunnel supervision code in
//! [`crate::runtime`] and the bounded job execution code in [`crate::jobs`]
//! cannot drift apart.

use std::process::{Child, Command};
use std::time::Duration;

use thiserror::Error;

/// How long a process group is given to exit after `SIGTERM` before it is
/// killed outright.
///
/// Only the Unix implementation sleep-waits, so the constant is unused when
/// compiling for Windows.
#[cfg_attr(not(unix), allow(dead_code))]
pub const TERM_GRACE: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to signal process group {pid}: {source}")]
    Signal { pid: u32, source: std::io::Error },
}

/// Starts every child in its own process group so that the whole tree can be
/// signalled later, even when the parent is only able to name the group leader.
pub fn detach_into_own_group(command: &mut Command) -> Result<(), ProcessError> {
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            // After `fork` but before `exec` the child is still single-threaded,
            // so it is safe to call `async-signal-safe` libc functions here.
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
    Ok(())
}

/// Spawn a child in its own process group. Keeping setup and spawn together
/// prevents callers from accidentally ignoring a failed group configuration.
pub fn spawn_in_own_group(command: &mut Command) -> std::io::Result<Child> {
    detach_into_own_group(command).map_err(|error| std::io::Error::other(error.to_string()))?;
    command.spawn()
}

/// Terminates the process group led by `pid`: `SIGTERM` first, then `SIGKILL`
/// if the group is still alive after [`TERM_GRACE`].
///
/// Signalling a group that has already vanished is not an error, which keeps
/// cleanup idempotent.
pub fn terminate_process_group(pid: u32) -> Result<(), ProcessError> {
    #[cfg(unix)]
    {
        if let Err(source) = signal_group(pid, libc::SIGTERM) {
            if !group_gone(&source) {
                return Err(ProcessError::Signal { pid, source });
            }
            return Ok(());
        }
        std::thread::sleep(TERM_GRACE);
        if let Err(source) = signal_group(pid, libc::SIGKILL) {
            if !group_gone(&source) {
                return Err(ProcessError::Signal { pid, source });
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status()
            .map_err(|source| ProcessError::Signal { pid, source })?;
        if status.success() {
            Ok(())
        } else {
            Err(ProcessError::Signal {
                pid,
                source: std::io::Error::other(format!("taskkill failed for pid {pid}")),
            })
        }
    }
}

/// Reports whether `pid` is still running.
pub fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // A signal-0 probe is not available, so ask the task list instead and
        // require an exact PID field match rather than a substring match.
        let filter = format!("PID eq {pid}");
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
        else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().any(|line| {
            line.split(',')
                .nth(1)
                .map(|field| field.trim().trim_matches('"') == pid.to_string())
                .unwrap_or(false)
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: libc::c_int) -> Result<(), std::io::Error> {
    unsafe {
        if libc::kill(-(pid as i32), signal) == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn group_gone(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_is_alive() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn a_reaped_pid_is_not_alive() {
        let mut child = shell_exit_0();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_alive(pid));
    }

    #[cfg(unix)]
    #[test]
    fn terminating_an_absent_group_is_not_an_error() {
        let mut child = shell_exit_0();
        let pid = child.id();
        child.wait().unwrap();
        // The group is gone, so the helper must report success.
        terminate_process_group(pid).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn detaching_starts_the_child_in_its_own_group() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("exit 0");
        let mut child = spawn_in_own_group(&mut command).unwrap();
        let pid = child.id();
        // The child is spawned into a fresh group whose id is its own pid. A
        // signal-0 probe therefore proves the group exists.
        assert_eq!(
            unsafe { libc::kill(-(pid as i32), 0) },
            0,
            "child must lead its own process group"
        );
        child.wait().unwrap();
    }

    fn shell_exit_0() -> std::process::Child {
        #[cfg(unix)]
        {
            Command::new("/bin/sh")
                .arg("-c")
                .arg("exit 0")
                .spawn()
                .unwrap()
        }
        #[cfg(windows)]
        {
            Command::new("cmd")
                .args(["/C", "exit", "0"])
                .spawn()
                .unwrap()
        }
    }
}
