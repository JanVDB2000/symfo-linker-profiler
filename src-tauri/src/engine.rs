use std::sync::OnceLock;

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

static CURRENT: OnceLock<Option<Engine>> = OnceLock::new();

/// The detected engine, probed once per process.
///
/// Detection spawns a subprocess, and the status poll runs repeatedly, so the result
/// is cached. Installing an engine while the app runs therefore needs a restart, which
/// is a fair trade against probing on every refresh.
pub fn current() -> Option<Engine> {
    *CURRENT.get_or_init(|| detect(crate::runtime::engine_responds))
}
