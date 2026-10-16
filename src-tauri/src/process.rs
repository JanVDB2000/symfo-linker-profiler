use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

/// Why a command produced no output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    /// The binary could not be started: usually not installed, or not in PATH.
    NotStarted,
    /// The command was still running when its deadline passed, and was killed.
    TimedOut,
    /// The process started but its result could not be collected.
    Interrupted,
}

/// How often the child is checked while waiting. Short enough to stay responsive,
/// long enough that waiting costs nothing measurable.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Runs `command` with an upper bound on how long it may take.
///
/// `Command::output` waits forever, which in a GUI means a container engine that hangs
/// takes the status panel with it. This kills the child once the deadline passes and
/// reports that as a distinct outcome, so the interface can say what happened.
///
/// Both pipes are drained on their own threads: a child that fills the pipe buffer
/// blocks on the write, so collecting output only after the wait would deadlock the
/// very command this is meant to bound.
pub fn output_within(mut command: Command, timeout: Duration) -> Result<Output, CommandError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|_| CommandError::NotStarted)?;
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                // Reap the child, so killing it does not leave a zombie behind.
                let _ = child.wait();
                return Err(CommandError::TimedOut);
            }
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CommandError::Interrupted);
            }
        }
    };

    Ok(Output {
        status,
        stdout: collect(stdout, deadline)?,
        stderr: collect(stderr, deadline)?,
    })
}

/// Finds an executable on `PATH`, trying the extensions this platform executes.
///
/// Resolution happens here rather than being left to the process launcher, because
/// the interesting tools are shell shims on Windows: `CreateProcess` cannot start a
/// `.bat`, and asking `cmd` to find one instead turns a missing program into a
/// locale-dependent error string. Knowing the full path is what says which form to
/// use, and whether to offer the tool at all.
pub fn find_on_path(path_value: &OsStr, name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    const EXTENSIONS: [&str; 4] = [".bat", ".cmd", ".exe", ""];
    #[cfg(not(windows))]
    const EXTENSIONS: [&str; 1] = [""];
    std::env::split_paths(path_value)
        .filter(|directory| !directory.as_os_str().is_empty())
        .flat_map(|directory| {
            EXTENSIONS
                .iter()
                .map(move |extension| directory.join(format!("{name}{extension}")))
        })
        .find(|candidate| candidate.is_file())
}

/// Whether a resolved binary is a shell shim that `CreateProcess` cannot start, and
/// that therefore has to be handed to `cmd /C` as an argument.
pub fn is_shim(binary: &Path) -> bool {
    binary.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
    })
}

/// Starts a program without waiting for it and without inheriting this app's streams.
///
/// An editor stays open for hours, so there is nothing to wait for and no output to
/// collect: the opposite of [`output_within`]. The child is still reaped on its own
/// thread, because a `Child` dropped without being waited for leaves a zombie behind
/// on Unix for as long as this app runs.
pub fn spawn_detached(mut command: Command) -> Result<(), CommandError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| CommandError::NotStarted)?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn drain(mut source: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        // Output that cannot be read is reported as empty: the exit status still stands.
        let _ = source.read_to_end(&mut buffer);
        let _ = sender.send(buffer);
    });
    receiver
}

fn collect(reader: Option<Receiver<Vec<u8>>>, deadline: Instant) -> Result<Vec<u8>, CommandError> {
    match reader {
        Some(receiver) => receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| CommandError::TimedOut),
        None => Ok(Vec::new()),
    }
}
