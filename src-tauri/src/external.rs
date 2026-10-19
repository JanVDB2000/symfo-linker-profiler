//! Handing a project directory to another tool: an editor, a terminal, a file manager.
//!
//! Nothing here reads or writes a project. It starts a program the user asked for, on
//! a directory this app resolved itself from a scan - never on a path the webview
//! supplied, because a command that accepted a path would be a way to start an
//! arbitrary program on an arbitrary directory.
//!
//! Which tools exist is a fixed catalogue per platform, and a tool is only offered
//! once its binary is actually found on `PATH`. The directory is always passed as an
//! argument or as the working directory, never interpolated into a command string: a
//! project path with a space, a bracket or an ampersand in it is an ordinary path,
//! not a quoting problem.

use crate::discovery::scan;
use crate::models::Message;
use crate::process::{find_on_path, is_shim, spawn_detached};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

/// How the directory reaches the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// As the last argument, after the fixed ones.
    Argument,
    /// Only as the working directory, for a tool that opens where it is started.
    WorkingDirectory,
}

/// One way of starting one tool. Several entries may share an `id`: the first whose
/// binary is installed wins, which is how Windows Terminal is preferred over a plain
/// console without the interface having to ask about it.
pub struct Tool {
    /// The stable id the interface sends back.
    pub id: &'static str,
    /// Binaries to look for, in order of preference.
    pub binaries: &'static [&'static str],
    /// Arguments that come before the directory.
    pub arguments: &'static [&'static str],
    pub pass: Pass,
}

/// Every id in the catalogue, in the order the interface shows them.
pub const IDS: [&str; 4] = ["phpstorm", "vscode", "terminal", "files"];

#[cfg(windows)]
pub const TOOLS: &[Tool] = &[
    Tool {
        id: "phpstorm",
        // The Toolbox shim and the direct installation, in that order.
        binaries: &["phpstorm", "phpstorm64"],
        arguments: &[],
        pass: Pass::Argument,
    },
    Tool {
        id: "vscode",
        binaries: &["code"],
        arguments: &[],
        pass: Pass::Argument,
    },
    Tool {
        id: "terminal",
        binaries: &["wt"],
        arguments: &["-d"],
        pass: Pass::Argument,
    },
    // Without Windows Terminal, `start` opens a console. It is a cmd builtin, so cmd
    // is the program; the directory it starts in is the working directory, which
    // keeps it out of the command line entirely.
    Tool {
        id: "terminal",
        binaries: &["cmd"],
        arguments: &["/C", "start", "", "cmd"],
        pass: Pass::WorkingDirectory,
    },
    Tool {
        id: "files",
        binaries: &["explorer"],
        arguments: &[],
        pass: Pass::Argument,
    },
];

#[cfg(target_os = "macos")]
pub const TOOLS: &[Tool] = &[
    Tool {
        id: "phpstorm",
        binaries: &["phpstorm"],
        arguments: &[],
        pass: Pass::Argument,
    },
    Tool {
        id: "vscode",
        binaries: &["code"],
        arguments: &[],
        pass: Pass::Argument,
    },
    Tool {
        id: "terminal",
        binaries: &["open"],
        arguments: &["-a", "Terminal"],
        pass: Pass::Argument,
    },
    Tool {
        id: "files",
        binaries: &["open"],
        arguments: &[],
        pass: Pass::Argument,
    },
];

#[cfg(all(unix, not(target_os = "macos")))]
pub const TOOLS: &[Tool] = &[
    Tool {
        id: "phpstorm",
        binaries: &["phpstorm", "phpstorm.sh"],
        arguments: &[],
        pass: Pass::Argument,
    },
    Tool {
        id: "vscode",
        binaries: &["code"],
        arguments: &[],
        pass: Pass::Argument,
    },
    // A terminal emulator opens in the directory it was started in; the alternative is
    // a different flag for every one of them.
    //
    // The Debian alternatives symlink stays first, because on a Debian or Ubuntu
    // machine it points at the terminal the user actually chose. It does not exist
    // everywhere though - NixOS has no alternatives system - so the desktop terminals
    // follow, and then the standalone ones that are often the only terminal installed.
    // First one found wins.
    Tool {
        id: "terminal",
        binaries: &[
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
            "alacritty",
            "kitty",
            "wezterm",
            "foot",
            "ghostty",
        ],
        arguments: &[],
        pass: Pass::WorkingDirectory,
    },
    Tool {
        id: "files",
        binaries: &["xdg-open"],
        arguments: &[],
        pass: Pass::Argument,
    },
];

/// A resolved program, its arguments and the directory it starts in.
#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub tool: &'static str,
    pub binary: String,
    pub arguments: Vec<String>,
    pub directory: PathBuf,
}

/// Builds the launch for one catalogue entry, or `None` when its binary is not
/// installed on this computer.
pub fn launch(tool: &Tool, directory: &Path, path_value: &OsStr) -> Option<Launch> {
    let binary = tool
        .binaries
        .iter()
        .find_map(|name| find_on_path(path_value, name))?;
    let mut arguments: Vec<String> = tool.arguments.iter().map(|&one| one.to_owned()).collect();
    if tool.pass == Pass::Argument {
        arguments.push(directory.to_string_lossy().into_owned());
    }
    // A `.cmd` shim - `code.cmd`, a Toolbox `phpstorm.cmd` - cannot be started by
    // CreateProcess, so cmd starts it with the resolved path as an argument.
    if is_shim(&binary) {
        let mut shimmed = vec!["/C".to_owned(), binary.to_string_lossy().into_owned()];
        shimmed.extend(arguments);
        return Some(Launch {
            tool: tool.id,
            binary: "cmd".to_owned(),
            arguments: shimmed,
            directory: directory.to_path_buf(),
        });
    }
    Some(Launch {
        tool: tool.id,
        binary: binary.to_string_lossy().into_owned(),
        arguments,
        directory: directory.to_path_buf(),
    })
}

/// The first entry for `id` whose binary is installed.
pub fn resolve(id: &str, directory: &Path, path_value: &OsStr) -> Option<Launch> {
    TOOLS
        .iter()
        .filter(|tool| tool.id == id)
        .find_map(|tool| launch(tool, directory, path_value))
}

/// Which tools are installed, in catalogue order. The interface offers these and
/// nothing else, so a button never promises a program this computer does not have.
pub fn available(path_value: &OsStr) -> Vec<&'static str> {
    IDS.iter()
        .copied()
        .filter(|id| resolve(id, Path::new("."), path_value).is_some())
        .collect()
}

/// Starts the tool and forgets about it: an editor outlives this app's interest in it.
pub fn start(launch: &Launch) -> Result<(), Message> {
    let mut command = Command::new(&launch.binary);
    command
        .current_dir(&launch.directory)
        .args(&launch.arguments);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    spawn_detached(command).map_err(|_| Message::new("This tool could not be started."))
}

/// Opens one project of the workspace in one tool.
pub fn open(development_root: &Path, project_id: &str, tool: &str) -> Result<(), Message> {
    open_with(development_root, project_id, tool, &mut start)
}

pub fn open_with(
    development_root: &Path,
    project_id: &str,
    tool: &str,
    launcher: &mut impl FnMut(&Launch) -> Result<(), Message>,
) -> Result<(), Message> {
    if !IDS.contains(&tool) {
        return Err(Message::new("Unknown tool."));
    }
    let root = development_root
        .canonicalize()
        .map_err(|_| Message::new("The development root does not exist or is not readable."))?;
    // The webview names a project, and the scan says where that project is.
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
    let path_value = std::env::var_os("PATH").unwrap_or_default();
    let launch = resolve(tool, Path::new(&project.path), &path_value).ok_or_else(|| {
        Message::new("This tool could not be found on this computer. Is it in PATH?")
    })?;
    launcher(&launch)
}
