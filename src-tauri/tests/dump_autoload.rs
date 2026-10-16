use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use symfolinker_lib::dump_autoload::{
    dump_with_output, host_command, run_host_command, tail, target, Target,
};
use symfolinker_lib::process::find_on_path;

const TIMEOUT: Duration = Duration::from_secs(30);

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-dump-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn project(&self, name: &str, compose: bool) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("composer.json"), r#"{"name":"acme/app"}"#).unwrap();
        if compose {
            fs::write(path.join("compose.yaml"), "services:\n  php-fpm: {}\n").unwrap();
        }
        path
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const CONFIG_JSON: &str = r#"{"services":{"php-fpm":{"volumes":[]},"database":{"volumes":[]}}}"#;

#[test]
fn runs_in_the_container_only_when_its_service_answers() {
    assert_eq!(
        target(Some("php-fpm".to_owned()), true, true),
        Target::Container("php-fpm".to_owned())
    );
    // Known but down: the host regenerates it, and the report says why.
    let Target::Host(Some(message)) = target(Some("php-fpm".to_owned()), true, false) else {
        panic!("expected the host with an explanation");
    };
    assert_eq!(
        message.key,
        "The PHP container is not running, so Composer ran on the host instead."
    );
    let Target::Host(Some(message)) = target(None, true, false) else {
        panic!("expected the host with an explanation");
    };
    assert_eq!(
        message.key,
        "No PHP service could be identified, so Composer ran on the host."
    );
    // A project without Compose runs natively, so the host is not a fallback and
    // there is nothing to explain.
    assert_eq!(target(None, false, false), Target::Host(None));
}

#[test]
fn keeps_the_tail_of_composer_output_within_bounds() {
    let output = (1..=40)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let kept = tail(&output);
    assert_eq!(kept.lines().count(), 20);
    assert!(kept.starts_with("line 21"), "{kept}");
    assert!(kept.ends_with("line 40"), "{kept}");
    // Blank lines are dropped rather than filling the budget.
    assert_eq!(
        tail("\n\nGenerated autoload files\n\n"),
        "Generated autoload files"
    );
    assert!(tail(&"x".repeat(10_000)).len() <= 4_000);
}

#[cfg(windows)]
const SHIM: &str = "composer.bat";
#[cfg(not(windows))]
const SHIM: &str = "composer";

#[test]
fn resolves_composer_on_path_before_a_project_phar() {
    let workspace = Workspace::new();
    let bin = workspace.0.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join(SHIM), "").unwrap();
    let path_value = OsString::from(bin.to_string_lossy().into_owned());
    assert_eq!(find_on_path(&path_value, "composer"), Some(bin.join(SHIM)));
    assert_eq!(find_on_path(&path_value, "no-such-tool"), None);

    let project = workspace.project("app", false);
    let (binary, args) = host_command(&project, Some(&path_value)).unwrap();
    // Windows resolves a .bat shim, which only cmd can start; everywhere else the
    // resolved binary is started directly.
    #[cfg(windows)]
    {
        assert_eq!(binary, "cmd");
        assert_eq!(args[0], "/C");
        assert_eq!(args[1], bin.join(SHIM).to_string_lossy());
        assert_eq!(args[2], "dump-autoload");
    }
    #[cfg(not(windows))]
    {
        assert_eq!(binary, bin.join(SHIM).to_string_lossy());
        assert_eq!(args, ["dump-autoload"]);
    }

    // Nothing on PATH: a project carrying its own Composer still has one.
    let empty = OsString::new();
    assert_eq!(host_command(&project, Some(&empty)), None);
    fs::write(project.join("composer.phar"), "").unwrap();
    assert_eq!(
        host_command(&project, Some(&empty)),
        Some((
            "php".to_owned(),
            vec!["composer.phar".to_owned(), "dump-autoload".to_owned()],
        ))
    );
}

#[cfg(windows)]
fn echo(text: &str) -> (String, Vec<String>) {
    (
        "cmd".to_owned(),
        vec!["/C".to_owned(), "echo".to_owned(), text.to_owned()],
    )
}
#[cfg(not(windows))]
fn echo(text: &str) -> (String, Vec<String>) {
    ("/bin/echo".to_owned(), vec![text.to_owned()])
}

#[cfg(windows)]
fn failing() -> (String, Vec<String>) {
    (
        "cmd".to_owned(),
        vec!["/C".to_owned(), "echo boom 1>&2 & exit /b 1".to_owned()],
    )
}
#[cfg(not(windows))]
fn failing() -> (String, Vec<String>) {
    (
        "/bin/sh".to_owned(),
        vec!["-c".to_owned(), "echo boom 1>&2; exit 1".to_owned()],
    )
}

#[test]
fn reports_composer_output_a_failure_detail_and_a_missing_binary() {
    let workspace = Workspace::new();
    let project = workspace.project("app", false);
    let output = run_host_command(&project, &echo("Generated autoload files"), TIMEOUT).unwrap();
    assert!(output.contains("Generated autoload files"), "{output}");

    let failure = run_host_command(&project, &failing(), TIMEOUT).unwrap_err();
    assert_eq!(failure.key, "Composer error: {detail}");
    assert_eq!(failure.params["detail"], "boom");

    let missing = run_host_command(
        &project,
        &("symfolinker-no-such-composer".to_owned(), Vec::new()),
        TIMEOUT,
    )
    .unwrap_err();
    assert_eq!(
        missing.key,
        "No Composer found. Is composer in PATH, or composer.phar in the project?"
    );
}

#[test]
fn regenerates_in_the_php_service_of_a_running_project() {
    let workspace = Workspace::new();
    workspace.project("app", true);
    let mut calls: Vec<String> = Vec::new();
    let report = dump_with_output(&workspace.0, "app", &mut |_, args, _| {
        calls.push(args.join(" "));
        Ok(match args {
            ["compose", "config", ..] => CONFIG_JSON.to_owned(),
            ["compose", "ps", ..] => "9f3c1ab0c0de\n".to_owned(),
            _ => "Generating optimized autoload files\nGenerated autoload files\n".to_owned(),
        })
    })
    .unwrap();
    assert_eq!(report.location, "container");
    assert_eq!(report.service.as_deref(), Some("php-fpm"));
    assert_eq!(
        report.output,
        "Generating optimized autoload files\nGenerated autoload files"
    );
    assert!(report.message.is_none());
    assert_eq!(
        calls.last().unwrap(),
        "compose exec -T php-fpm composer dump-autoload"
    );
    // A dump writes nothing of this app's own: Composer only rewrites what is already
    // inside vendor/composer.
    assert!(!workspace.0.join("app/vendor").exists());
}

#[test]
fn an_unknown_project_is_refused_before_composer_is_started() {
    let workspace = Workspace::new();
    workspace.project("app", true);
    let error = dump_with_output(&workspace.0, "library/app", &mut |_, _, _| {
        panic!("nothing may run for a project that is not in the workspace")
    })
    .unwrap_err();
    assert_eq!(error.key, "Project {id} is no longer in this workspace.");
    assert_eq!(error.params["id"], "library/app");
}
