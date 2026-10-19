use crate::atomic::write_atomically;
use crate::errors::SymfoLinkerError;
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// A lock older than this, from a holder that cannot answer, may be taken over.
/// Comfortably above the longest operation the app runs: a `composer dump-autoload`
/// is bounded at three minutes.
pub const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// How long an acquire waits for *this* process to finish with the same root.
///
/// Long enough to absorb the brief overlaps the app creates itself - the mount
/// synchronisation that follows a switch is the common one - and short enough that a
/// genuinely long operation reports "still running" instead of appearing to hang.
const IN_PROCESS_WAIT: Duration = Duration::from_secs(2);

/// How often the in-process registry is re-checked while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// What a failure to take the operating system lock actually means.
///
/// `fs4` reports contention as `WouldBlock` and everything else as an I/O error, and
/// collapsing the two is what made a filesystem that cannot lock at all report as a
/// second copy of this app. The distinction is kept in a pure function so every
/// branch is testable without an exotic filesystem to hand.
#[derive(Debug)]
pub enum LockOutcome {
    /// Something else holds the lock.
    Held,
    /// This filesystem does not implement file locking. Nothing holds anything.
    Unsupported,
    /// A real failure, which keeps its own wording.
    Failed(io::Error),
}

// 33 (ERROR_LOCK_VIOLATION) never reaches here: fs4 maps it to WouldBlock. 32 does.
#[cfg(windows)]
const HELD_CODES: &[i32] = &[32];
// ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER: what the
// \\wsl.localhost 9P redirector and some network shares answer LockFileEx with.
#[cfg(windows)]
const UNSUPPORTED_CODES: &[i32] = &[1, 50, 87];

// EAGAIN, for the platforms where it arrives without an ErrorKind.
#[cfg(unix)]
const HELD_CODES: &[i32] = &[libc::EAGAIN, libc::EWOULDBLOCK];
// ENOLCK, EINVAL, EOPNOTSUPP: NFS without a lock daemon, and some FUSE mounts.
#[cfg(unix)]
const UNSUPPORTED_CODES: &[i32] = &[libc::ENOLCK, libc::EINVAL, libc::EOPNOTSUPP, libc::ENOTSUP];

/// Decides what a failed lock attempt means. See [`LockOutcome`].
pub fn classify(error: &io::Error) -> LockOutcome {
    if error.kind() == io::ErrorKind::WouldBlock {
        return LockOutcome::Held;
    }
    match error.raw_os_error() {
        Some(code) if HELD_CODES.contains(&code) => LockOutcome::Held,
        Some(code) if UNSUPPORTED_CODES.contains(&code) => LockOutcome::Unsupported,
        _ => LockOutcome::Failed(duplicate(error)),
    }
}

/// `io::Error` is not `Clone`, and the original is borrowed from the caller.
fn duplicate(error: &io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(code) => io::Error::from_raw_os_error(code),
        None => io::Error::from(error.kind()),
    }
}

/// Who holds the lock, as far as it could be recorded.
///
/// This lives beside the lock rather than inside it, so the lock file stays an empty
/// sentinel whose only meaning is the operating system lock on the open handle.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub pid: u32,
    pub host: String,
    /// Distinguishes one run of this program from another on the same machine,
    /// which a pid alone cannot do: pids are reused.
    pub instance: String,
    pub operation: String,
    /// Whole seconds since the Unix epoch. Deliberately not a formatted date:
    /// staleness is integer arithmetic, with no timezone or parsing to get wrong.
    pub acquired_at: u64,
}

/// Whether a held lock has an owner worth believing.
#[derive(Debug, PartialEq)]
pub enum HeldVerdict {
    LiveOwner,
    Abandoned { minutes: u64 },
}

/// Judges a held lock's owner record.
///
/// There is no process-liveness check here, and that is an honest limit rather than
/// an omission: across a Windows/WSL boundary a recorded pid belongs to a different
/// namespace and means nothing, and pids are reused in any case. So the question
/// answered is the narrower one this can actually answer - is this record old, from
/// this machine, and not ours - and everything else is left alone.
pub fn judge_owner(
    record: Option<&Owner>,
    now: u64,
    our_instance: &str,
    our_host: &str,
) -> HeldVerdict {
    let Some(record) = record else {
        // The lock is held and nothing claims it: a leftover from a version that
        // wrote no record, or a record that never reached the disk.
        return HeldVerdict::Abandoned { minutes: 0 };
    };
    // Our own nonce on a held lock means this process still holds it. A long
    // dump-autoload looks exactly like this, and taking over from ourselves is a bug.
    if record.instance == our_instance {
        return HeldVerdict::LiveOwner;
    }
    // Another machine's lock cannot be judged from here: its clock is not ours.
    if record.host != our_host {
        return HeldVerdict::LiveOwner;
    }
    let age = now.saturating_sub(record.acquired_at);
    if age < STALE_AFTER.as_secs() {
        return HeldVerdict::LiveOwner;
    }
    HeldVerdict::Abandoned { minutes: age / 60 }
}

/// Roots whose filesystem turned out not to support locking, so the interface can say
/// so once it is known. Populated on the first attempt and never cleared: the
/// filesystem does not change under one run of the app.
fn degraded() -> &'static Mutex<HashSet<PathBuf>> {
    static DEGRADED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    DEGRADED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// True when this root's filesystem cannot lock files, so a second instance working
/// in it could not be detected. Read by the scanner and by the write checks.
pub fn locking_unsupported(development_root: &Path) -> bool {
    // Canonical, to match how the root was keyed when the lock was taken.
    let key = development_root
        .canonicalize()
        .unwrap_or_else(|_| development_root.to_path_buf());
    degraded().lock().is_ok_and(|roots| roots.contains(&key))
}

/// Development roots this process is currently working in.
fn busy_roots() -> &'static Mutex<HashSet<PathBuf>> {
    static BUSY: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    BUSY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Serialises this process against itself, before the operating system lock is tried.
///
/// The operating system lock is per handle - `LockFileEx` on Windows, `flock` per open
/// file description on Unix - so two overlapping commands inside one app conflict with
/// each other, and that used to be reported as a second instance. Queueing is the
/// honest answer, because the overlap is this app's own doing.
#[derive(Debug)]
struct InProcessGuard(PathBuf);

impl Drop for InProcessGuard {
    fn drop(&mut self) {
        if let Ok(mut roots) = busy_roots().lock() {
            roots.remove(&self.0);
        }
    }
}

fn acquire_in_process(development_root: &Path) -> Option<InProcessGuard> {
    let deadline = Instant::now() + IN_PROCESS_WAIT;
    loop {
        if let Ok(mut roots) = busy_roots().lock() {
            if roots.insert(development_root.to_path_buf()) {
                return Some(InProcessGuard(development_root.to_path_buf()));
            }
        }
        // Bounded, never indefinite: a frozen window is worse than a clear message.
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Identifies this run of this program, for the life of the process.
fn instance() -> &'static str {
    static INSTANCE: OnceLock<String> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0);
        format!("{}-{nanos}", std::process::id())
    })
}

/// This machine's name, used only to refuse to judge another machine's lock.
fn host() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        if let Ok(name) = std::env::var("COMPUTERNAME") {
            return name;
        }
        if let Ok(name) = std::env::var("HOSTNAME") {
            return name;
        }
        // HOSTNAME is a shell variable and is often not exported to a child process,
        // so on Unix the file is the reliable source.
        fs::read_to_string("/etc/hostname")
            .map(|name| name.trim().to_owned())
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "unknown".to_owned())
    })
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// Exclusive lock on a development root, held for the duration of a mutation.
///
/// Plan section 32: one global lock per development root is enough to stop two swaps
/// from touching the same vendor directory at once. The lock lives in
/// `<root>/.symfolinker/lock` (hard rule 2.2: app state sits above the projects,
/// never inside one), with the owner record beside it.
///
/// Mutating functions take `&RootLock` as a parameter, so holding the lock is a
/// compile-time requirement rather than a convention someone can forget.
#[derive(Debug)]
pub struct RootLock {
    file: File,
    path: PathBuf,
    root: PathBuf,
    locking_supported: bool,
    /// Released on drop, after the operating system lock.
    _in_process: InProcessGuard,
}

impl RootLock {
    pub fn acquire(development_root: &Path) -> Result<Self, SymfoLinkerError> {
        Self::acquire_for(development_root, "an operation")
    }

    /// As [`acquire`](Self::acquire), recording what the lock is held for so an
    /// abandoned lock can say what left it behind.
    pub fn acquire_for(development_root: &Path, operation: &str) -> Result<Self, SymfoLinkerError> {
        Self::lock(development_root, operation)
    }

    /// Retries acquiring a lock whose owner record appears abandoned.
    ///
    /// The operating system releases a dead holder's lock. Always reuse the sentinel:
    /// an old or absent owner record cannot prove that a holder has stopped working,
    /// and unlinking a locked sentinel on Unix would bypass that holder's lock.
    pub fn take_over(development_root: &Path) -> Result<Self, SymfoLinkerError> {
        Self::lock(development_root, "a recovered lock")
    }

    fn lock(development_root: &Path, operation: &str) -> Result<Self, SymfoLinkerError> {
        // Key the in-process guard on the canonical path, never on what the caller
        // happened to pass. Call sites disagree - `config::set_php_service` passes the
        // root as typed while `container::sync_mounts` canonicalises first - and two
        // spellings of one directory would otherwise each get their own guard and sail
        // straight into fighting each other over the one file lock. That pair is
        // exactly the overlap this guard exists to stop.
        let key = development_root
            .canonicalize()
            .unwrap_or_else(|_| development_root.to_path_buf());
        let in_process = acquire_in_process(&key).ok_or_else(|| SymfoLinkerError::LockBusy {
            path: development_root.to_string_lossy().into_owned(),
        })?;

        let directory = Self::state_directory(development_root)?;
        let path = directory.join("lock");
        if fs::symlink_metadata(&path)
            .is_ok_and(|meta| crate::discovery::is_link(&meta) || !meta.is_file())
        {
            return Err(SymfoLinkerError::WriteDenied {
                path: path.to_string_lossy().into_owned(),
            });
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| SymfoLinkerError::io(&path, &error))?;

        // try_lock, never a blocking lock: the wait above is bounded, and a frozen
        // window is worse than a clear message. Called through the trait explicitly,
        // because newer Rust versions add an inherent File::try_lock that would
        // otherwise shadow it with other semantics.
        let mut locking_supported = true;
        if let Err(error) = FileExt::try_lock(&file).map_err(io::Error::from) {
            match classify(&error) {
                LockOutcome::Held => return Err(Self::held_error(development_root)),
                LockOutcome::Unsupported => {
                    locking_supported = false;
                    if let Ok(mut roots) = degraded().lock() {
                        roots.insert(key.clone());
                    }
                }
                LockOutcome::Failed(inner) => return Err(SymfoLinkerError::io(&path, &inner)),
            }
        }

        // Best effort: the record is diagnostic, and failing to write it must not stop
        // work the lock itself has already permitted.
        let _ = Self::write_owner(development_root, operation);

        Ok(Self {
            file,
            path,
            root: development_root.to_path_buf(),
            locking_supported,
            _in_process: in_process,
        })
    }

    /// `<root>/.symfolinker`, created if absent and refused if it is not a directory.
    fn state_directory(development_root: &Path) -> Result<PathBuf, SymfoLinkerError> {
        let directory = development_root.join(".symfolinker");
        if fs::symlink_metadata(&directory)
            .is_ok_and(|meta| !meta.is_dir() || crate::discovery::is_link(&meta))
        {
            return Err(SymfoLinkerError::WriteDenied {
                path: directory.to_string_lossy().into_owned(),
            });
        }
        fs::create_dir_all(&directory).map_err(|error| SymfoLinkerError::io(&directory, &error))?;
        Ok(directory)
    }

    /// Distinguishes a lock that is genuinely busy from one nobody is coming back for.
    fn held_error(development_root: &Path) -> SymfoLinkerError {
        let path = development_root.to_string_lossy().into_owned();
        let record = Self::read_owner(development_root);
        match judge_owner(record.as_ref(), now_seconds(), instance(), host()) {
            HeldVerdict::LiveOwner => SymfoLinkerError::LockUnavailable { path },
            HeldVerdict::Abandoned { minutes } => SymfoLinkerError::LockStale {
                path,
                minutes: minutes.to_string(),
            },
        }
    }

    pub fn owner_path(development_root: &Path) -> PathBuf {
        development_root.join(".symfolinker").join("owner.json")
    }

    fn read_owner(development_root: &Path) -> Option<Owner> {
        fs::read(Self::owner_path(development_root))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    fn write_owner(development_root: &Path, operation: &str) -> Result<(), SymfoLinkerError> {
        let record = Owner {
            pid: std::process::id(),
            host: host().to_owned(),
            instance: instance().to_owned(),
            operation: operation.to_owned(),
            acquired_at: now_seconds(),
        };
        let path = Self::owner_path(development_root);
        let json = serde_json::to_vec(&record).map_err(|error| SymfoLinkerError::IoError {
            path: path.to_string_lossy().into_owned(),
            detail: error.to_string(),
        })?;
        write_atomically(&path, &json)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// False when this root's filesystem could not lock, so the lock is this process's
    /// own discipline and nothing more.
    pub fn locking_supported(&self) -> bool {
        self.locking_supported
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        // The record goes first: a lock about to be released must not leave an owner
        // behind for the next acquire to judge.
        let _ = fs::remove_file(Self::owner_path(&self.root));
        // Best effort: the handle closing releases the lock regardless.
        let _ = FileExt::unlock(&self.file);
    }
}
