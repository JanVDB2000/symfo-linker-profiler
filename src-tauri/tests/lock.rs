//! The development-root lock: what it refuses, what it tolerates, and what it
//! admits it cannot know.
//!
//! The two judgements the lock makes are pure functions, and that is deliberate:
//! one decides what an operating system error *means*, the other decides whether
//! an absent owner is dead. Neither can be reached honestly through a real lock
//! from a single test process - the first needs a filesystem that cannot lock,
//! the second needs a second process that died - so both are tested directly and
//! their use in `acquire` is left trivial.

use std::{
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};
use symfolinker_lib::errors::SymfoLinkerError;
use symfolinker_lib::lock::{
    classify, judge_owner, HeldVerdict, LockOutcome, Owner, RootLock, STALE_AFTER,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A development root, cleaned up on drop.
struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-lock-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// classify: what an operating system error means
// ---------------------------------------------------------------------------

#[test]
fn a_would_block_error_is_contention() {
    let error = io::Error::from(io::ErrorKind::WouldBlock);

    assert!(matches!(classify(&error), LockOutcome::Held));
}

#[cfg(windows)]
#[test]
fn a_sharing_violation_is_contention_too() {
    // fs4 maps only ERROR_LOCK_VIOLATION to WouldBlock, so ERROR_SHARING_VIOLATION
    // arrives here raw and would otherwise read as a broken filesystem.
    let error = io::Error::from_raw_os_error(32);

    assert!(matches!(classify(&error), LockOutcome::Held));
}

#[cfg(windows)]
#[test]
fn a_filesystem_that_cannot_lock_is_not_another_instance() {
    // What a \\?\UNC\wsl.localhost\... root returns: the 9P redirector does not
    // implement byte-range locking, and reporting that as a second instance is
    // the bug this closes.
    for code in [1, 50, 87] {
        let error = io::Error::from_raw_os_error(code);

        assert!(
            matches!(classify(&error), LockOutcome::Unsupported),
            "os error {code} must read as a filesystem that cannot lock"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_filesystem_that_cannot_lock_is_not_another_instance() {
    // ENOLCK, EINVAL, EOPNOTSUPP: NFS without a lock daemon, and some FUSE mounts.
    for code in [libc::ENOLCK, libc::EINVAL, libc::EOPNOTSUPP, libc::ENOTSUP] {
        let error = io::Error::from_raw_os_error(code);

        assert!(
            matches!(classify(&error), LockOutcome::Unsupported),
            "errno {code} must read as a filesystem that cannot lock"
        );
    }
}

#[test]
fn an_unexpected_failure_keeps_its_own_wording() {
    let error = io::Error::from(io::ErrorKind::PermissionDenied);

    match classify(&error) {
        LockOutcome::Failed(inner) => {
            assert_eq!(inner.kind(), io::ErrorKind::PermissionDenied);
        }
        other => panic!("a permission failure must stay itself, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// judge_owner: whether a held lock has an owner worth believing
// ---------------------------------------------------------------------------

const HOST: &str = "test-host";
const OURS: &str = "instance-ours";
const THEIRS: &str = "instance-theirs";

fn owner(host: &str, instance: &str, acquired_at: u64) -> Owner {
    Owner {
        pid: 4321,
        host: host.to_owned(),
        instance: instance.to_owned(),
        operation: "a switch".to_owned(),
        acquired_at,
    }
}

#[test]
fn a_fresh_record_from_another_instance_is_a_live_owner() {
    let record = owner(HOST, THEIRS, 1_000);

    let verdict = judge_owner(Some(&record), 1_060, OURS, HOST);

    assert!(matches!(verdict, HeldVerdict::LiveOwner));
}

#[test]
fn an_old_record_from_this_machine_is_abandoned() {
    let record = owner(HOST, THEIRS, 1_000);
    let now = 1_000 + STALE_AFTER.as_secs() + 60;

    match judge_owner(Some(&record), now, OURS, HOST) {
        HeldVerdict::Abandoned { minutes } => {
            assert_eq!(minutes, (STALE_AFTER.as_secs() + 60) / 60);
        }
        other => panic!("an old local record must be abandoned, got {other:?}"),
    }
}

#[test]
fn an_old_record_from_another_machine_is_left_alone() {
    // Another machine's lock cannot be judged from here: its pid means nothing and
    // its clock is not ours. Saying "abandoned" would be a guess with a
    // destructive button attached to it.
    let record = owner("some-other-host", THEIRS, 1_000);
    let now = 1_000 + STALE_AFTER.as_secs() + 60;

    let verdict = judge_owner(Some(&record), now, OURS, HOST);

    assert!(matches!(verdict, HeldVerdict::LiveOwner));
}

#[test]
fn our_own_record_is_never_abandoned_however_old() {
    // Our own nonce on a held lock means this process still holds it, and a long
    // dump-autoload is exactly that. Taking over from ourselves would be a bug.
    let record = owner(HOST, OURS, 1_000);
    let now = 1_000 + STALE_AFTER.as_secs() * 10;

    let verdict = judge_owner(Some(&record), now, OURS, HOST);

    assert!(matches!(verdict, HeldVerdict::LiveOwner));
}

#[test]
fn a_missing_record_on_a_held_lock_is_abandoned() {
    // Nothing wrote an owner, yet the lock is held: a pre-upgrade leftover, or a
    // record that never reached the disk. There is nothing here to believe in.
    match judge_owner(None, 1_000, OURS, HOST) {
        HeldVerdict::Abandoned { minutes } => assert_eq!(minutes, 0),
        other => panic!("an ownerless held lock must be abandoned, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// One process never fights itself
// ---------------------------------------------------------------------------

#[test]
fn an_overlapping_acquire_in_one_process_waits_instead_of_failing() {
    // The bug this closes: two overlapping commands inside one app reported
    // "another SymfoLinker instance is working in this development root".
    let workspace = Workspace::new();
    let root = workspace.0.clone();

    let held = RootLock::acquire(&root).unwrap();
    let worker = thread::spawn(move || RootLock::acquire(&root));

    thread::sleep(Duration::from_millis(100));
    drop(held);

    assert!(
        worker.join().unwrap().is_ok(),
        "a queued acquire must succeed once the holder releases"
    );
}

#[test]
fn an_acquire_that_waits_too_long_says_so_plainly() {
    let workspace = Workspace::new();
    let _held = RootLock::acquire(&workspace.0).unwrap();

    let error = RootLock::acquire(&workspace.0).unwrap_err();

    // Not LockUnavailable: there is no second instance, and naming one sent the
    // user hunting for something that does not exist.
    assert_eq!(
        error,
        SymfoLinkerError::LockBusy {
            path: workspace.0.to_string_lossy().into_owned()
        }
    );
}

// ---------------------------------------------------------------------------
// The ownership record
// ---------------------------------------------------------------------------

#[test]
fn holding_the_lock_records_who_holds_it() {
    let workspace = Workspace::new();

    let lock = RootLock::acquire(&workspace.0).unwrap();

    let record: Owner =
        serde_json::from_slice(&fs::read(RootLock::owner_path(&workspace.0)).unwrap()).unwrap();
    assert_eq!(record.pid, std::process::id());
    assert!(!record.instance.is_empty());
    assert!(record.acquired_at > 0);
    // The lock file itself stays an empty sentinel: the record is a sibling.
    // Read through the metadata, not the contents: the lock covers the whole byte
    // range, so on Windows even this process cannot read the bytes it has locked.
    assert_eq!(fs::metadata(lock.path()).unwrap().len(), 0);
}

#[test]
fn the_record_lives_above_the_projects_beside_the_lock() {
    let workspace = Workspace::new();
    let project = workspace.0.join("app");
    fs::create_dir_all(&project).unwrap();

    let lock = RootLock::acquire(&workspace.0).unwrap();

    // Hard rule 2.2: app state sits above the projects, never inside one.
    assert_eq!(
        RootLock::owner_path(&workspace.0),
        workspace.0.join(".symfolinker").join("owner.json")
    );
    assert_eq!(lock.path(), workspace.0.join(".symfolinker").join("lock"));
    assert!(!project.join(".symfolinker").exists());
}

#[test]
fn releasing_the_lock_clears_the_record() {
    let workspace = Workspace::new();

    drop(RootLock::acquire(&workspace.0).unwrap());

    assert!(
        !RootLock::owner_path(&workspace.0).exists(),
        "a released lock must not leave an owner behind to be judged later"
    );
}

#[test]
fn a_released_lock_can_be_acquired_again() {
    let workspace = Workspace::new();
    drop(RootLock::acquire(&workspace.0).unwrap());

    assert!(RootLock::acquire(&workspace.0).is_ok());
}

// ---------------------------------------------------------------------------
// Taking over
// ---------------------------------------------------------------------------

#[test]
fn taking_over_replaces_an_abandoned_record() {
    let workspace = Workspace::new();
    let directory = workspace.0.join(".symfolinker");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("owner.json"),
        serde_json::to_vec(&owner("some-host", "a-dead-instance", 1)).unwrap(),
    )
    .unwrap();

    let lock = RootLock::take_over(&workspace.0).unwrap();

    let record: Owner =
        serde_json::from_slice(&fs::read(RootLock::owner_path(&workspace.0)).unwrap()).unwrap();
    assert_eq!(record.pid, std::process::id());
    drop(lock);
}

#[test]
fn taking_over_is_refused_while_this_process_still_holds_the_lock() {
    // Takeover exists for a holder that cannot answer. Ours can.
    let workspace = Workspace::new();
    let _held = RootLock::acquire(&workspace.0).unwrap();

    assert_eq!(
        RootLock::take_over(&workspace.0).unwrap_err(),
        SymfoLinkerError::LockBusy {
            path: workspace.0.to_string_lossy().into_owned()
        }
    );
}

#[test]
fn taking_over_preserves_an_os_lock_regardless_of_its_owner_record() {
    use fs4::FileExt;

    for record in [
        None,
        Some(owner(HOST, THEIRS, 1)),
        Some(owner(HOST, THEIRS, u64::MAX)),
    ] {
        let workspace = Workspace::new();
        let directory = workspace.0.join(".symfolinker");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        // A raw handle models another holder without entering our process registry.
        FileExt::try_lock(&file).unwrap();
        let owner_path = RootLock::owner_path(&workspace.0);
        if let Some(record) = record {
            fs::write(&owner_path, serde_json::to_vec(&record).unwrap()).unwrap();
        }
        let before = fs::read(&owner_path).ok();

        assert!(matches!(
            RootLock::take_over(&workspace.0),
            Err(SymfoLinkerError::LockStale { .. } | SymfoLinkerError::LockUnavailable { .. })
        ));
        assert_eq!(fs::read(&owner_path).ok(), before);
        assert!(RootLock::acquire(&workspace.0).is_err());

        drop(file);
        assert!(RootLock::take_over(&workspace.0).is_ok());
    }
}
