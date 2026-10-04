use std::{
    io::Read,
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
