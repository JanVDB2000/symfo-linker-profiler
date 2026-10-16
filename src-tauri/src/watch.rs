//! Noticing a change instead of asking for one every five minutes.
//!
//! A `composer install` that writes over a link, and a branch that moves under a
//! linked checkout, are both worth seeing immediately: they are exactly the moments
//! the replaced-link notice and the branch column stop telling the truth. Polling
//! finds them eventually; a watcher finds them now.
//!
//! Two things keep that from becoming expensive. Every watch is non-recursive, on the
//! three places where those changes actually land, so a dependency tree of thousands
//! of files is never traversed. And a burst of events is debounced into one change,
//! because an install writes for a while and a single refresh afterwards is the whole
//! point.

use crate::models::{Message, Project};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

/// How long the writing has to stop before a burst counts as finished.
pub const DEBOUNCE: Duration = Duration::from_millis(700);

/// What changed under a project, and therefore what has to be read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    /// Something in `.git`: a checkout, a commit, a staged file. The project's status
    /// is stale, which one cheap refresh answers.
    Branch,
    /// Something under `vendor`: an install, or a link replaced by a directory. Only
    /// a scan can describe that correctly, so it outranks a branch change.
    Vendor,
}

fn rank(change: Change) -> u8 {
    match change {
        Change::Branch => 0,
        Change::Vendor => 1,
    }
}

/// The directories worth watching for one project, as they exist right now.
///
/// Non-recursive, all of them: a recursive watch on a project would report every file
/// an IDE touches, and a recursive watch on `vendor` would report every file of every
/// dependency - the cost this is meant to avoid. Three places matter. `.git`, where a
/// checkout lands. `vendor/composer`, which an install rewrites. And the vendor
/// directory of each mapped package, where a link is replaced by a directory.
pub fn targets(project: &Project) -> Vec<PathBuf> {
    let root = PathBuf::from(&project.path);
    let mut targets = vec![root.join(".git"), root.join("vendor").join("composer")];
    targets.extend(
        project
            .packages
            .iter()
            .filter_map(|package| Path::new(&package.vendor_path).parent())
            .map(Path::to_path_buf),
    );
    targets.sort();
    targets.dedup();
    targets.retain(|target| target.is_dir());
    targets
}

/// Which kind of change a reported path is, or `None` for a path that is neither.
///
/// The decision is made on the path's own components rather than on a prefix of the
/// project, because a watcher reports the path the operating system used - which on
/// Windows is not the extended-length form a canonicalised project path carries. Only
/// the directories above are watched, so a reported path is always inside one of them.
pub fn classify(path: &Path) -> Option<Change> {
    let mut vendor = false;
    for component in path.components() {
        match component.as_os_str().to_string_lossy().as_ref() {
            ".git" => return Some(Change::Branch),
            "vendor" => vendor = true,
            _ => {}
        }
    }
    vendor.then_some(Change::Vendor)
}

/// The strongest change in a burst of paths.
pub fn strongest(paths: &[PathBuf]) -> Option<Change> {
    paths
        .iter()
        .filter_map(|path| classify(path))
        .max_by_key(|change| rank(*change))
}

fn stronger(current: Option<Change>, next: Option<Change>) -> Option<Change> {
    match (current, next) {
        (Some(current), Some(next)) if rank(next) > rank(current) => Some(next),
        (Some(current), _) => Some(current),
        (None, next) => next,
    }
}

fn cannot_watch() -> Message {
    Message::new("Changes to this project cannot be watched; status is polled instead.")
}

/// A live watch. Dropping it stops the watcher, which ends its worker thread.
pub struct Watch {
    _watcher: RecommendedWatcher,
    _worker: thread::JoinHandle<()>,
}

/// Watches one project and reports debounced changes until the handle is dropped.
///
/// A watch that cannot be established is an error rather than a silent nothing: the
/// interface keeps polling in that case, and saying so is what makes the difference
/// visible instead of mysterious.
pub fn watch(
    project: &Project,
    debounce: Duration,
    mut on_change: impl FnMut(Change) + Send + 'static,
) -> Result<Watch, Message> {
    let targets = targets(project);
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // An event that only reads something changes nothing worth refreshing for.
        if let Ok(event) = event {
            if !matches!(event.kind, EventKind::Access(_)) {
                let _ = sender.send(event.paths);
            }
        }
    })
    .map_err(|_| cannot_watch())?;
    let watched = targets
        .iter()
        .filter(|target| watcher.watch(target, RecursiveMode::NonRecursive).is_ok())
        .count();
    if watched == 0 {
        return Err(cannot_watch());
    }
    let worker = thread::spawn(move || collect(receiver, debounce, &mut on_change));
    Ok(Watch {
        _watcher: watcher,
        _worker: worker,
    })
}

/// Turns bursts of events into single changes.
///
/// Ends by itself: dropping the watcher closes the sender, and the first `recv` after
/// that fails. The window starts at the first event of a burst, so a long install
/// reports once while it runs and once more when it finishes, rather than thousands
/// of times or only once it is over.
fn collect(
    receiver: Receiver<Vec<PathBuf>>,
    debounce: Duration,
    on_change: &mut impl FnMut(Change),
) {
    while let Ok(paths) = receiver.recv() {
        let mut change = strongest(&paths);
        let deadline = Instant::now() + debounce;
        while let Ok(more) =
            receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            change = stronger(change, strongest(&more));
        }
        if let Some(change) = change {
            on_change(change);
        }
    }
}
