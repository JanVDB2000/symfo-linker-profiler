use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// The container engine SymfoLinker talks to.
///
/// Podman ships a Compose-compatible CLI, so every subcommand this app uses
/// (`compose ps`, `compose config`, `compose exec`) takes identical arguments. Only
/// the binary name differs, which is the entire difference handled here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Docker,
    Podman,
}

impl Engine {
    pub fn binary(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }

    /// Shown in the interface, so the user knows which engine answered.
    pub fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
        }
    }
}

/// Picks the first engine whose CLI answers, Docker first.
///
/// `probe` reports whether a binary is usable, which keeps the choice testable
/// without either engine installed.
pub fn detect(probe: impl Fn(&str) -> bool) -> Option<Engine> {
    [Engine::Docker, Engine::Podman]
        .into_iter()
        .find(|engine| probe(engine.binary()))
}

/// How long a failed detection is believed before probing again.
///
/// Long enough that repeated status polls do not spawn a process every time, short
/// enough that starting Docker Desktop is noticed without restarting the app.
const RETRY_AFTER: Duration = Duration::from_secs(30);

struct Cached {
    engine: Option<Engine>,
    probed_at: Instant,
}

static CURRENT: Mutex<Option<Cached>> = Mutex::new(None);

/// The detected engine.
///
/// Detection spawns a subprocess and the status poll runs repeatedly, so the result is
/// cached. A found engine is kept for the life of the process: the binary does not move,
/// and whether its daemon answers is a separate question the status check asks every
/// time anyway. A failed detection expires, because the usual reason for one is an
/// engine that has not been started yet, and needing a restart to notice that is exactly
/// the problem this expiry removes.
pub fn current() -> Option<Engine> {
    let mut cached = CURRENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = cached.as_ref() {
        if previous.engine.is_some() || previous.probed_at.elapsed() < RETRY_AFTER {
            return previous.engine;
        }
    }
    let engine = detect(crate::runtime::engine_responds);
    *cached = Some(Cached {
        engine,
        probed_at: Instant::now(),
    });
    engine
}

/// Drops the cached result, so the next call probes again.
///
/// Scanning the workspace calls this: an explicit refresh is the moment to stop
/// believing anything measured earlier.
pub fn forget() {
    *CURRENT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}
