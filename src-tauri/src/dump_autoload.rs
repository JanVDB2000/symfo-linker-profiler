//! Regenerating a project's autoloader, which is the repair for the one problem the
//! autoload comparison can only report.
//!
//! This is the single place where SymfoLinker runs Composer itself, and it runs
//! exactly one subcommand. `dump-autoload` installs nothing, resolves nothing and
//! changes no manifest: it rewrites the generated files in `vendor/composer/` from
//! what is already on disk. That is what separates it from `composer install` and
//! `composer update`, which this app must never run to repair a situation.
//!
//! Where it runs matters as much as what it runs. An autoloader is read by the PHP
//! that uses it, so when a project has a PHP service whose container is running, the
//! autoloader is regenerated in that container. Otherwise it is regenerated on the
//! host, and the report says which happened rather than leaving it to be guessed.

use crate::compose::{parse_compose_config, suggest_php_service};
use crate::config;
use crate::discovery::{compose_file, scan};
use crate::lock::RootLock;
use crate::models::{AutoloadDump, Message, Project};
use crate::process::{find_on_path, is_shim, output_within, CommandError};
use crate::runtime::container_output_with_timeout;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Regenerating an autoloader walks every autoloaded file in the project, which on a
/// large one takes minutes rather than seconds. The bound exists for a Composer that
/// stopped making progress, not for a slow one.
const DUMP_TIMEOUT: Duration = Duration::from_secs(180);

/// Reading the Compose configuration, as everywhere else in this app.
const CONFIG_TIMEOUT: Duration = Duration::from_secs(20);

/// How many lines of Composer's own output are kept. Its last words are the
/// interesting ones: what it generated, or what it refused to.
const MAX_OUTPUT_LINES: usize = 20;

/// A hard cap beside the line count, so one enormous line cannot travel over IPC.
const MAX_OUTPUT_BYTES: usize = 4_000;

/// Which PHP regenerates the autoloader.
#[derive(Debug, PartialEq)]
pub enum Target {
    /// The named Compose service, whose container is running.
    Container(String),
    /// The host's own Composer, with the reason the container was not used when one
    /// could reasonably have been expected.
    Host(Option<Message>),
}

/// Decides where Composer has to run, given what the project has and what answered.
///
/// Pure on purpose: the choice is the part worth being certain about, and the part a
/// test can pin down without a container engine in the room.
pub fn target(service: Option<String>, has_compose: bool, running: bool) -> Target {
    match service {
        Some(service) if running => Target::Container(service),
        // A service that is known but not running: the host still has a Composer, and
        // using it beats refusing. Saying which one ran is the whole point.
        Some(_) => Target::Host(Some(Message::new(
            "The PHP container is not running, so Composer ran on the host instead.",
        ))),
        None if has_compose => Target::Host(Some(Message::new(
            "No PHP service could be identified, so Composer ran on the host.",
        ))),
        // A project without Compose runs natively; the host is not a fallback there.
        None => Target::Host(None),
    }
}

/// Keeps the tail of Composer's output, bounded in lines and in bytes.
///
/// Composer's text is source data, like the container engine's: it is passed through
/// verbatim rather than translated, because rewriting a tool's own words is how a
/// message stops matching what the tool actually said.
pub fn tail(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    let first = lines.len().saturating_sub(MAX_OUTPUT_LINES);
    let kept = lines[first..].join("\n");
    if kept.len() <= MAX_OUTPUT_BYTES {
        return kept;
    }
    let start = (kept.len() - MAX_OUTPUT_BYTES..kept.len())
        .find(|index| kept.is_char_boundary(*index))
        .unwrap_or(kept.len());
    kept[start..].to_owned()
}

/// The command that regenerates the autoloader on the host, or `None` when no Composer
/// is reachable at all.
///
/// A `.bat` or `.cmd` shim is handed to `cmd /C` as an argument, never interpolated
/// into a command string, so a path with spaces or brackets in it stays one argument.
/// A project carrying its own `composer.phar` is the fallback, which is how plenty of
/// repositories keep Composer per project.
pub fn host_command(project: &Path, path_value: Option<&OsStr>) -> Option<(String, Vec<String>)> {
    if let Some(binary) = path_value.and_then(|value| find_on_path(value, "composer")) {
        let shim = is_shim(&binary);
        let binary = binary.to_string_lossy().into_owned();
        return Some(if shim {
            (
                "cmd".to_owned(),
                vec!["/C".to_owned(), binary, "dump-autoload".to_owned()],
            )
        } else {
            (binary, vec!["dump-autoload".to_owned()])
        });
    }
    project.join("composer.phar").is_file().then(|| {
        (
            "php".to_owned(),
            vec!["composer.phar".to_owned(), "dump-autoload".to_owned()],
        )
    })
}

fn missing_composer() -> Message {
    Message::new("No Composer found. Is composer in PATH, or composer.phar in the project?")
}

/// Runs a resolved host command in the project directory, bounded like every other
/// subprocess this app starts, and without flashing a console window.
pub fn run_host_command(
    project: &Path,
    command: &(String, Vec<String>),
    timeout: Duration,
) -> Result<String, Message> {
    let mut process = Command::new(&command.0);
    process.current_dir(project).args(&command.1);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    let output = output_within(process, timeout).map_err(|failure| match failure {
        CommandError::NotStarted => missing_composer(),
        CommandError::TimedOut => {
            let mut message = Message::new("Composer did not finish within {seconds} seconds.");
            message
                .params
                .insert("seconds".to_owned(), timeout.as_secs().to_string());
            message
        }
        CommandError::Interrupted => Message::new("Composer returned an unknown error."),
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    // Composer writes its diagnostics to stderr and its progress to stdout, so a
    // failure is explained by the first and only then by the second.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr
        .lines()
        .chain(stdout.lines())
        .find(|line| !line.trim().is_empty());
    Err(match detail {
        Some(detail) => Message::with("Composer error: {detail}", "detail", detail.trim()),
        None => Message::new("Composer returned an unknown error."),
    })
}

/// The PHP service of a project: the stored choice, or the one its Compose file
/// suggests. `None` when the project has no Compose file, or nothing PHP-shaped in it.
fn php_service(
    development_root: &Path,
    project: &Project,
    run: &mut impl FnMut(&Path, &[&str], Duration) -> Result<String, Message>,
) -> Option<String> {
    let project_path = Path::new(&project.path);
    compose_file(project_path)?;
    if let Some(stored) = config::php_service(development_root, &project.path) {
        return Some(stored);
    }
    let stdout = run(
        project_path,
        &["compose", "config", "--format", "json"],
        CONFIG_TIMEOUT,
    )
    .ok()?;
    suggest_php_service(&parse_compose_config(&stdout))
}

/// Whether the service's container is up, which is what makes an exec possible.
fn service_running(
    project: &Path,
    service: &str,
    run: &mut impl FnMut(&Path, &[&str], Duration) -> Result<String, Message>,
) -> bool {
    run(project, &["compose", "ps", "-q", service], CONFIG_TIMEOUT)
        .is_ok_and(|stdout| stdout.lines().any(|line| !line.trim().is_empty()))
}

/// Regenerates the autoloader of one project and reports where that happened.
///
/// Ids come from the webview and paths never do: the project is resolved by scanning
/// the development root, exactly like a switch. The development-root lock is held for
/// the duration, because a swap moving directories under a running `dump-autoload`
/// would produce an autoloader describing neither state.
pub fn dump(development_root: &Path, project_id: &str) -> Result<AutoloadDump, Message> {
    dump_with_output(
        development_root,
        project_id,
        &mut container_output_with_timeout,
    )
}

pub fn dump_with_output(
    development_root: &Path,
    project_id: &str,
    run: &mut impl FnMut(&Path, &[&str], Duration) -> Result<String, Message>,
) -> Result<AutoloadDump, Message> {
    let root = development_root
        .canonicalize()
        .map_err(|_| Message::new("The development root does not exist or is not readable."))?;
    let project = scan(&root)?
        .projects
        .into_iter()
        .find(|candidate| candidate.id == project_id)
        .ok_or_else(|| {
            Message::with(
                "Project {id} is no longer in this workspace.",
                "id",
                project_id.to_owned(),
            )
        })?;
    let _lock = RootLock::acquire(&root).map_err(|error| error.message())?;
    let project_path = PathBuf::from(&project.path);
    let service = php_service(&root, &project, run);
    let running = service
        .as_deref()
        .is_some_and(|service| service_running(&project_path, service, run));
    match target(service, compose_file(&project_path).is_some(), running) {
        Target::Container(service) => {
            let stdout = run(
                &project_path,
                &[
                    "compose",
                    "exec",
                    "-T",
                    &service,
                    "composer",
                    "dump-autoload",
                ],
                DUMP_TIMEOUT,
            )?;
            Ok(AutoloadDump {
                location: "container",
                service: Some(service),
                output: tail(&stdout),
                message: None,
            })
        }
        Target::Host(message) => {
            let command = host_command(&project_path, std::env::var_os("PATH").as_deref())
                .ok_or_else(missing_composer)?;
            let stdout = run_host_command(&project_path, &command, DUMP_TIMEOUT)?;
            Ok(AutoloadDump {
                location: "host",
                service: None,
                output: tail(&stdout),
                message,
            })
        }
    }
}
