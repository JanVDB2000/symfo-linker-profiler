use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use symfolinker_lib::external::{available, open_with, resolve, Launch};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "symfolinker-external-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn project(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("composer.json"), r#"{"name":"acme/app"}"#).unwrap();
        path
    }
    /// A directory holding one fake editor binary, used as the whole of PATH.
    fn bin(&self, name: &str) -> OsString {
        let bin = self.0.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(name), "").unwrap();
        OsString::from(bin.to_string_lossy().into_owned())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
const EDITOR: &str = "code.cmd";
#[cfg(not(windows))]
const EDITOR: &str = "code";

#[test]
fn offers_only_the_tools_this_computer_actually_has() {
    let workspace = Workspace::new();
    let path_value = workspace.bin(EDITOR);
    assert_eq!(available(&path_value), ["vscode"]);
    assert_eq!(available(&OsString::new()), Vec::<&str>::new());
}

#[test]
fn passes_a_directory_with_a_space_as_one_argument() {
    let workspace = Workspace::new();
    let path_value = workspace.bin(EDITOR);
    let directory = workspace.0.join("my projects/app");
    let launch = resolve("vscode", &directory, &path_value).unwrap();
    assert_eq!(launch.tool, "vscode");
    assert_eq!(launch.directory, directory);
    // On Windows the editor is a .cmd shim, which only cmd can start; the resolved
    // path and the directory stay separate arguments either way.
    #[cfg(windows)]
    {
        assert_eq!(launch.binary, "cmd");
        assert_eq!(launch.arguments[0], "/C");
        assert!(launch.arguments[1].ends_with("code.cmd"));
        assert_eq!(
            launch.arguments[2],
            directory.to_string_lossy().into_owned()
        );
    }
    #[cfg(not(windows))]
    {
        assert!(launch.binary.ends_with("code"));
        assert_eq!(launch.arguments, [directory.to_string_lossy().into_owned()]);
    }
    // A tool nothing on this PATH provides is not resolved into a guess.
    assert!(resolve("phpstorm", &directory, &path_value).is_none());
}

#[test]
fn opens_the_path_the_scan_found_for_the_named_project() {
    let workspace = Workspace::new();
    let project = workspace.project("library/bundle");
    let mut opened: Vec<PathBuf> = Vec::new();
    // PATH is the process's own here, so this only resolves where an editor or file
    // manager exists; the assertion that matters is which directory was handed over.
    let result = open_with(&workspace.0, "library/bundle", "files", &mut |launch| {
        opened.push(launch.directory.clone());
        Ok(())
    });
    match result {
        Ok(()) => {
            assert_eq!(opened.len(), 1);
            assert_eq!(
                opened[0].canonicalize().unwrap(),
                project.canonicalize().unwrap()
            );
        }
        // No file manager on this machine's PATH: the refusal must come from the
        // catalogue rather than from a half-built launch.
        Err(message) => assert_eq!(
            message.key,
            "This tool could not be found on this computer. Is it in PATH?"
        ),
    }
}

#[test]
fn refuses_an_unknown_tool_and_an_unknown_project_without_starting_anything() {
    let workspace = Workspace::new();
    workspace.project("app");
    let mut refuse = |_: &Launch| -> Result<(), symfolinker_lib::models::Message> {
        panic!("nothing may be started for input that was refused")
    };
    let unknown_tool = open_with(&workspace.0, "app", "notepad", &mut refuse).unwrap_err();
    assert_eq!(unknown_tool.key, "Unknown tool.");
    let unknown_project = open_with(&workspace.0, "library/app", "files", &mut refuse).unwrap_err();
    assert_eq!(
        unknown_project.key,
        "Project {id} is no longer in this workspace."
    );
    assert_eq!(unknown_project.params["id"], "library/app");
    let unknown_root = open_with(
        Path::new("D:/no-such-root-symfolinker"),
        "app",
        "files",
        &mut refuse,
    )
    .unwrap_err();
    assert_eq!(
        unknown_root.key,
        "The development root does not exist or is not readable."
    );
}
