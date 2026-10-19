# User guide

[Back to the README](../README.md) | [Development guide](development.md)

Installation, workspace setup, package switching and recovery.

## Contents

- [Installation](#installation)
- [The development root](#the-development-root)
- [The interface](#the-interface)
- [Languages](#languages)
- [Backups, profiles and containers](#backups-profiles-and-containers)
- [Scan rules](#scan-rules)
- [Status values](#status-values)
- [Common messages](#common-messages)

## Installation

Most people do not need this repository. Grab an installer from the [latest release](https://github.com/JanVDB2000/symfo-linker-profiler/releases/latest) instead:

| Platform | File |
| --- | --- |
| Windows | `.msi` or `.exe` setup |
| macOS | `.dmg` (separate builds for Apple silicon and Intel) |
| Linux | `.AppImage`, `.deb` (Debian/Ubuntu) or `.rpm` (Fedora/RHEL) |
| NixOS | Built from source: `nix profile install github:JanVDB2000/symfo-linker-profiler/v0.5.0` |

Releases also contain `SymfoLinker_*.app.tar.gz`. Those are macOS update bundles, not downloads: use the `.dmg`.

To modify SymfoLinker, follow the [development guide](development.md#getting-started).

### These builds are not signed

Release binaries are unsigned, so Windows and macOS may display security warnings. Only open a download if you trust its source.

- **Windows** shows "Windows protected your PC". Choose *More info*, then *Run anyway*.
- **macOS** says the app "cannot be opened because the developer cannot be verified". Right-click the app, choose *Open*, then confirm; or run `xattr -dr com.apple.quarantine /Applications/SymfoLinker.app`.
- **Linux** AppImages need the executable bit: `chmod +x SymfoLinker_*.AppImage`.
- **NixOS** cannot run any of those downloads. An installer carries the glibc and the
  WebKitGTK of the machine it was built on, and on NixOS neither sits where the binary
  looks for it. Install from the flake instead, which builds against this machine's own
  nixpkgs: `nix profile install github:JanVDB2000/symfo-linker-profiler/v0.5.0`. The
  question of trusting an unsigned binary does not arise there, because the build is
  your own.

If you would rather not trust an unsigned binary, building from source is the alternative, and every release is built from this repository by [the release workflow](../.github/workflows/release.yml).

## The development root

The development root is the directory **above** your projects. SymfoLinker matches a dependency to a local project by comparing the `name` in `composer.json` with requirements in the other projects.

```text
dev/                       # ← choose this directory
  app/
    composer.json          # require: { "acme/bundle": "^1.0" }
    vendor/
      acme/bundle          # vendor directory or link to ../../../bundle
      .symfolinker/        # the preserved Composer package while the local version is active
  bundle/
    composer.json          # name: "acme/bundle"
```

In this example, `app` gets one local dependency: `acme/bundle`, with `bundle/` as its source.

### Link chains

Projects may use one another in a chain. A project can have multiple locally linked packages at once, and a linked project can contain links of its own:

```text
dev/
  a/vendor/acme/b -> ../../../b      a links acme/b
  a/vendor/acme/d -> ../../../d      a also links acme/d
  d/vendor/acme/c -> ../../../c      d itself links acme/c
```

Each project is inspected at its own path, so all three links are reported independently as `local`. The Projects panel shows **Used by** for each project, while **Chain** in Active links shows the links made by the target project. Both can be clicked to follow the chain.

## The interface

The interface follows the **Symfony profiler**: a compact header with project search, a status bar with a colored top border, a collector menu as the navigation column, and panels with compact metrics and tables using monospace values. Normal content width is 1200 px, the navigation column is 220 px, and the gap between menu and panel is 30 px.

Typography follows the profiler too: system fonts at 14 px for the interface, JetBrains Mono at 13 px for table values and paths, and panel titles at 24 px with weight 500. JetBrains Mono is bundled locally in `public/fonts/`, including its license.

| Component | Contents |
| --- | --- |
| **Status bar** | Status code (`OK`, `LOCAL` or `ERROR`), scan state, root path, selected project and runtime. Errors turn the whole bar red. |
| **Collector menu** | Five panels with project count, active link count and health issue count. |
| **Project picker** | Filter field above a scrolling list, with a marker for a dirty working directory. The header search and this field share one filter, matching project and Composer package names. |
| **Workspace** | Path input, directory picker and scan button above the collector menu; expandable through Workspace after scanning. |
| **Panel layout** | Metrics at the top, then key/value property tables and the package table. |

The panels:

| Panel | Contents |
| --- | --- |
| **Projects** | Composer package, path, branch, commit, working directory and container status, with a freshness badge and a Sync button; all locally available dependencies with mode, branch, host status, backup status and expandable paths. The Mode column is a `VENDOR` / `LOCAL` toggle; switching asks for confirmation first. A package carries a label when its link was replaced from outside, when the checkout provides a version this project does not ask for, or when it autoloads differently than the generated autoloader. **Undo last switch** appears beside the table after a switch, and **Regenerate the autoloader** appears above it when a checkout autoloads differently. An **Open in** row hands the project directory to an editor, a terminal or the file manager, or copies its path. |
| **Active links** | One block per local package: its branch and changed-file count, the local source path, every project linking to it, and its onward chain. Package and project names navigate. |
| **Runtime** | Docker Compose or native runtime, detected Compose file, host path and live container status. |
| **Health** | Recovery of interrupted swaps, write checks, the preserved versions of this project with their size, root readability, project count, Git information availability and package issues. |
| **Settings** | Language, theme, scan scope, change detection and project access. |

### Opening a project in another tool

The **Projects** panel has an **Open in** row: the project directory can be handed to
PhpStorm, VS Code, a terminal or the file manager, and **Copy path** puts the path on
the clipboard.

Only tools this computer actually has are offered. Each one is looked up on `PATH`
when the interface opens - the Toolbox shim for PhpStorm, `code` for VS Code, Windows
Terminal with a console as its fallback, `explorer`, `open` or `xdg-open` for the file
manager - so a button never promises a program that is not installed. If none is
found, the row says so instead of showing dead buttons.

The interface sends the project id, never a path: the directory is resolved from a
fresh scan, and it reaches the program as an argument or as its working directory
rather than inside a command string. A project path with a space, a bracket or an
ampersand in it is an ordinary path here.

### Noticing changes instead of polling

While a project is selected, SymfoLinker watches three directories under it: `.git`,
`vendor/composer` and the vendor directory of each mapped package. A `composer install`
that writes over a link, and a branch that moves under a linked checkout, are noticed
as they happen rather than up to five minutes later.

- Every watch is **non-recursive**, so a dependency tree of thousands of files is never
  traversed. A burst of events is debounced into one change, because an install writes
  for a while and one refresh afterwards is the point.
- A change under `vendor` triggers a **scan**, since only a scan can describe a
  replaced link. A change under `.git` triggers the cheaper **status refresh**.
- While the watch holds, the five-minute poll stops. When a watch cannot be
  established - a network share, a platform limit - the poll stays, and
  **Settings → Environment → Change detection** says which of the two is in use.
- Watching stops while the window is hidden, like polling, and the status line carries
  a **watching** label while it is active.

On first launch, the theme follows the system preference (`prefers-color-scheme`), including changes during use. The switch below the menu toggles light and dark; **Settings → Appearance** also offers Automatic. Next to the theme switch, you can select normal or full width. Both choices are saved locally. The layout shrinks below 1200 px, and below 760 px the collector menu becomes horizontal.

## Languages

Open **Settings → Appearance → Language** and select **English (EN)**, **Nederlands (NL)**, **Français (FR)** or **Deutsch (DE)**. English is the default. Changes apply immediately without restarting or rescanning and are saved as `symfolinker.language`. The document's `lang` attribute and time formatting follow the selected language.

## Backups, profiles and containers

Switching to **LOCAL** preserves the Composer package in `vendor/.symfolinker/` and links your checkout. Switching to **VENDOR** restores the preserved package. Projects stay read-only outside `vendor/`; app locks, service settings and generated Compose overrides live above the projects in `<root>/.symfolinker/`. Project Composer and Compose files are unchanged.

Failed steps attempt rollback; external filesystem changes or I/O errors can prevent recovery. See the [safety model](development.md#safety-model) for implementation safeguards.

### Limits worth knowing

- Backups from older versions without a journal are marked **Unrecognized**. They can be restored explicitly, but their provenance cannot be verified.
- Scanning reports readability and path status only, because it never writes. Write permissions, the link kind this project would get and the free space are answered by **Health > Run write checks**, which does write - and removes what it wrote.
- The version and autoload comparisons describe the checkout, not the running application. They are read from `composer.json` and `vendor/composer/installed.json`, and both stay silent when they cannot prove anything.
- A recovery journal records the project, package, installed version (when known), timestamp and operation state. Filesystem inspection remains the source of truth for active modes; journal metadata is not a content checksum.

### What it writes, and where

Projects are read-only outside `vendor/`. SymfoLinker writes in exactly two places:

| Location | Contents |
| --- | --- |
| `<project>/vendor/` | The package link, preserved Composer directory, earlier versions under `vendor/.symfolinker/.history/`, recovery journal (`vendor/.symfolinker/state.json`), temporarily staged restore links, and the probes the write checks create and remove again (`.preflight-*`). |
| `<root>/.symfolinker/` | `lock` (one mutation at a time) and `config.json` (the PHP service per project), plus generated container-mount overrides in `compose/<project>/<service>.json`. Above the projects, never inside one. |

The last scanned root, theme, layout width and language are remembered in local webview storage (`localStorage` keys `symfolinker.root`, `symfolinker.theme`, `symfolinker.width` and `symfolinker.language`), rather than in a project file.

The scan result itself is cached there too (`symfolinker.scan`, plus the selected project in `symfolinker.project`), so reopening the app shows the same workspace without walking the filesystem again. The summary bar marks restored data as **Cached** and shows when it was scanned; **Refresh** reads the filesystem anew, and every switch replaces the stored copy with the state the backend read back. The cache belongs to the root it was taken in and is discarded when the root changes, when the entry is unreadable, incomplete or from an older version, when a scan fails, or when **Settings → Saved scan → Clear saved scan** is used. Because the cache describes a moment, not the present, anything live (Git status, containers) is still read on demand.

### Saved workspaces, profiles and recovery

Successful desktop scans automatically add their root to **Settings > Saved workspaces**. Open a saved root to scan it, or remove its shortcut. Shortcuts live in local webview storage (`symfolinker.workspaces`); removing one does not change any project files.

In **Settings > Link profiles**, save all current package modes under a name. Saving requires a fresh scan with no unknown package modes. Profiles are scoped to the development root and stored locally (`symfolinker.profiles`); saving the same name replaces that profile. Applying opens a confirmation listing every project, package and target mode. The backend takes one root lock, resolves the entire profile from a fresh scan and validates it before switching. If a later switch fails, completed changes are reversed in reverse order. Rollback can itself fail on external filesystem changes or I/O errors; the error identifies the affected path, and the interface automatically refreshes the workspace after errors. If that refresh also fails, projects stay visible but filesystem switches are blocked until Refresh succeeds.

**Health > Recover interrupted swaps** reconciles the selected project's journal with disk. A preserved backup at an empty vendor path is restored, even when the local checkout is gone. A broken local link is restored to the preserved Composer package, including when its checkout has disappeared. A completed working local switch is marked complete; a completed vendor restore clears the record and removes any staged link. Conflicting occupied paths are refused. Recovery is explicit: scanning never repairs or writes project files. A malformed journal is reported and preserved rather than overwritten.

Restoring VENDOR first verifies the backup, then stages the original link with a rename. If restoring fails, that exact link is moved back, including a broken symlink or junction. This preserves the local state without having to reconstruct a target that no longer exists. New journal records identify their project and package; legacy unrecorded backups remain visibly unrecognized.

Git and container requests are scoped to the current workspace and project. Late responses from an earlier selection are discarded, and changing projects starts its own status refresh immediately. After a swap error, the workspace is automatically read back without losing the selected project. If readback fails too, its data remains visible as stale, the disk cache is cleared and further filesystem switches require a successful Refresh.

### Composer reinstall and missing container mounts

If `composer install` replaces a local link with a fresh vendor directory while an earlier backup remains, switching back to LOCAL archives the earlier directory under `vendor/.symfolinker/.history/<vendor>/<package>/`. The newly installed Composer directory becomes the active backup. Switching to VENDOR restores that newest version. Both versions are preserved; a failed link step rolls the directory moves back. Each archived directory has a JSON sidecar with its recorded package/project/version metadata. Archives are never automatically deleted; **Health > Preserved versions** lists them and is the only place anything is removed by hand.

**An existing backup never blocks a switch to LOCAL.** Whatever occupies the backup path is archived first, in whatever state it is in: a finished backup, an unrecognized one from an older version, a half-written swap, a stray file, even a link. A link is moved as a link, so what it pointed at stays untouched, and nothing is deleted to make room. Refusing used to leave no way forward from inside the app for a backup location that had gone strange.

Restoring is the opposite case and still refuses: switching to VENDOR has to know that the directory it moves back really is the preserved Composer version, so a missing, invalid or foreign-owned backup stops it while the local link stays in place.

An application-only mount such as `/home/user/dev/app:/application` does not expose sibling repositories: `vendor/acme/bundle -> ../../../bundle` resolves to `/bundle` inside the container, where nothing is mounted. SymfoLinker therefore keeps the container's bind mounts in step with the links on disk, by itself.

**Mounts follow every switch.** After a switch to LOCAL, VENDOR, a profile or a recovery, SymfoLinker recomputes the complete set of mounts the packages that are LOCAL *right now* require. A package that became LOCAL gains the bind mount its source needs; a package back on VENDOR loses that mount again, because the generated override is rewritten to the whole desired set rather than appended to. Nothing is asked and nothing is confirmed: the mount is part of the switch.

It verifies the running container's project, service, original Compose files and custom environment files from its labels, writes the JSON Compose override above the projects, validates the combined configuration, and runs `compose up -d --no-deps --no-build --pull never --force-recreate` for the PHP service only. Project Compose files remain unchanged. A mounted path already occupied by another source is refused.

**Switches made in quick succession share one pass.** The synchronisation waits briefly for the next switch, so linking three bundles in a row recreates the container once instead of three times. Switching stays available during that wait and is held only while the pass itself runs, which is the one moment the development root's lock is taken.

**The service is recreated only when the set actually changed.** Switching packages a container already sees -- a workspace mounted whole, for instance -- restarts nothing. The same holds for a project without a Compose file, without an identifiable PHP service, or whose container is not running: those are answered without touching anything, and a project without Compose costs no process at all.

When the running container cannot be verified (for example, a Podman provider without compatible labels), the automatic pass changes nothing and the Runtime panel lists the paths under **Missing local source mounts**, with **Apply mounts and recreate PHP service** to apply them after a confirmation.

Docker cannot attach or detach a bind mount on an already-running container in place; a normal restart does not apply new mounts. The service must be recreated. Its image is reused as is -- no image is built or pulled -- and so are its mounted volumes, but data stored only in the container's writable layer is not preserved by recreation. Removing the last mount empties the override rather than deleting it, so an external `docker compose up` that already references the file keeps resolving. Later external `compose up` commands must include the generated override to retain these additional mounts; the next switch restores them if they were omitted.

### Links a composer install replaced

Installing dependencies writes a fresh Composer directory over a linked package. Nothing warns you: the application silently uses the installed version again, while the journal still records a finished link. SymfoLinker reads exactly that combination - a recorded link with a Composer directory in its place - and reports it as a **replaced link**, with a notice above the panels naming every package it found.

**Restore replaced links** switches them all back to LOCAL in one pass, across projects, because one `composer install` rarely stops at one package. It runs as a profile does: one lock on the development root, every package validated before anything moves, and completed changes reversed when a later one fails. The version Composer just installed becomes the preserved backup, and the version it replaced moves into history, so both remain available.

### Undo last switch

After a successful switch, a profile or a restore of replaced links, **Undo last switch** appears beside the package table. It puts back the modes the packages had a moment ago, through the same validated path as a profile, so it is a switch in reverse rather than a shortcut past the checks.

It describes a state of disk this session verified, which is why it disappears when a switch fails, when the development root changes, when the saved scan is cleared, and after the undo itself. A recovery clears it too: a repair is not a switch.

### Preserved versions

**Health > Show preserved versions** reads what is kept under `vendor/.symfolinker/.history/` for the selected project: the package, the version recorded when it was set aside, when that happened, its size and file count, and the total above the table. Reading it walks the directory and measures every archive, so it happens on request rather than on every scan.

**Restore** makes an archived version the active backup again, which is what a switch to VENDOR will put back. The version it replaces is archived in turn, so nothing is lost, and the link itself is untouched: only what is preserved behind it changes. This is only possible while the package is LOCAL - on VENDOR the Composer directory is the one in use, and exchanging what is preserved underneath it would change nothing visible.

**Remove** deletes one archived directory for good. It is the only thing in SymfoLinker that deletes anything: never automatic, never a repair for an error, always one archive you named, and always something SymfoLinker itself moved there. The package in use and the version preserved now are not touched. An entry that is not a package directory - a stray file or a link that was moved out of the way - is listed so it can be removed, and refused for restoring.

### Write checks

A switch moves directories and creates a link, and each of those can fail: a read-only vendor directory, a Windows installation without Developer Mode, a full disk. Scanning cannot answer any of it, because the only honest way to know whether a write works is to write.

**Health > Run write checks** therefore writes a probe in `<root>/.symfolinker/` and in `<project>/vendor/.symfolinker/`, creates a link there through the same code a real switch uses, and removes all three again. It reports whether both locations are writable, whether this project would get a relative symlink or a junction, the free space on the project's filesystem, and what the preserved versions already occupy. A failure carries its own wording rather than only a red cross.

It is a button and never automatic, because everything that happens by itself in this app reads. A project without a `vendor/` directory is reported as not switchable rather than having one created for it.

### What the checkout provides

A symlink bypasses Composer's resolver completely, so two mismatches that Composer would have refused can only be noticed afterwards. Both are reported as labels on the package, and both stay silent unless they can be established with certainty:

- **Version conflict** - the checkout declares a version line (an explicit `version`, or a `branch-alias` for its current branch) that the project does not ask for. `^2.0` against a 3.x checkout is a conflict; a plain `dev-main` branch without an alias proves nothing and is left alone.
- **Autoload differs** - the checkout declares different autoload rules than the ones the generated autoloader in `vendor/composer/` was built from. A moved source directory or a renamed namespace produces "class not found" with every other status green. Run `composer dump-autoload` in the project after switching.

## Scan rules

The scan is deliberately strict and predictable:

- A project is any directory holding a `composer.json`, wherever it sits: the root may be flat, grouped by team or client, or a mix of both.
- Descend **at most four directories** below the root. The fifth level is not read, which is what keeps a root chosen one directory too high from becoming a walk of the whole drive.
- **Stop at a project.** A directory that holds a `composer.json` is never descended into, so the packages in its `vendor/` and the manifests inside its tests are not projects of this workspace.
- Never enter `vendor`, `node_modules`, `target` or `dist`.
- Stop after 4000 directories **with** a warning, rather than scanning without a bound.
- Identify a project by its path below the root (`library/bundle`) and name it after its own directory (`bundle`). Two groups may therefore hold a project of the same name, and the picker lists each project under its group.
- Skip names beginning with a dot.
- Do not treat a symlink or junction directory as a project.
- The canonical path must stay inside the root; otherwise skip it.
- A directory without `composer.json` is not a project.
- Skip unreadable or invalid JSON manifests **with** a warning.
- Package names must have the form `vendor/package`, using lowercase letters, digits and `_ . -`, starting with an alphanumeric character and not ending with a dot. Platform requirements (`php`, `ext-json`) and traversal attempts (`../bundle`, `acme/..`, `C:/bundle`) are excluded from mapping.
- If multiple projects share a package name, skip the mapping with a warning rather than guessing. The candidates are reported so the choice can be made by hand; see [More than one checkout](#more-than-one-checkout).
- Do not map a project to itself.
- **Multiple and chained links** are supported. Each project is inspected at its own path: `a` can link both `acme/b` and `acme/d`, while `d` links `acme/c`. Links do not interfere because the scanner never traverses a package link to inspect another project.
- Sort projects by their path below the root, which keeps a group's projects together, and packages by package name.
- Read Git information only when the project itself has `.git`, preventing a development-root repository from being reported as the project repository. Invoke Git with `--no-optional-locks`, without terminal prompts and without console windows on Windows. A missing current branch is displayed as `detached HEAD`.
- Detect `compose.yaml`, `compose.yml`, `docker-compose.yaml` and `docker-compose.yml`.

## Status values

### Package mode

| Mode | Meaning |
| --- | --- |
| `vendor` | A real directory occupies the vendor path: the Composer version is active. |
| `local` | The vendor path is a link resolving to the expected local project. |
| `unknown` | Any other situation; consult link status for the reason. |

### Link status

| Status | English UI label | Meaning |
| --- | --- | --- |
| `notLinked` | Vendor directory | Original vendor directory is present. |
| `linked` | Local source reachable | Link points to the expected local source. |
| `broken` | Broken link | Link exists but its target does not. |
| `unexpectedTarget` | Unexpected link target | Link resolves somewhere other than the expected local project. |
| `missing` | Vendor package missing | Nothing occupies the vendor path. |
| `invalid` | Invalid path | Not a directory or link (for example, a file), a linked ancestor directory, or an unreadable path. |

### Backup status

| Status | Meaning |
| --- | --- |
| `missing` | No backup at `vendor/.symfolinker/<vendor>/<package>`. |
| `available` | A real backup directory exists and the journal records a completed local switch. |
| `invalid` | The backup path is unsafe or its recorded project/package does not match. |
| `interrupted` | A journal entry is still backing up or restoring; use Health to recover. |
| `unrecognized` | A backup directory has no readable journal entry, such as a legacy backup. |
| `lost` | A completed local switch was recorded but its backup is now missing. |

A package is additionally reported as a **replaced link** when a completed local switch was recorded and a Composer directory now occupies the vendor path: nothing in this app moved it there, so an external `composer install` did.

### Version comparison

What the local checkout provides, against the constraint this project requires. A checkout provides a version line when it declares `version`, or a `branch-alias` for the branch it is on.

| Status | Meaning |
| --- | --- |
| `satisfied` | The version line fits the constraint. |
| `mismatch` | The two cannot overlap, such as `^2.0` against a 3.x checkout. |
| `unknown` | It cannot be established either way: a `dev-main` branch without an alias, a stability flag, a hyphen range, a four-part version, or no declared version at all. |

A symlink bypasses Composer's resolver, so a `mismatch` never blocks a switch. It reports what Composer would have refused.

### Autoload comparison

The autoload rules the checkout declares, against the ones `vendor/composer/installed.json` recorded for that package - which is what the generated autoloader was built from. Paths are normalised first, so `src/` and `src` compare as equal.

| Status | Meaning |
| --- | --- |
| `matches` | The checkout autoloads the way the generated autoloader expects. |
| `differs` | Rules were moved, renamed or added. **Regenerate the autoloader** repairs this; see below. |
| `unknown` | One of the two sides is missing, such as a package installed from a path repository, or a checkout that declares no autoload rules. |

#### Regenerating the autoloader

**Projects** shows a notice above the dependency table listing every package whose
autoload rules differ, with **Regenerate the autoloader** beside it. It runs
`composer dump-autoload` for the selected project and then reads the workspace again,
because the label it clears is read from disk rather than remembered.

Where it runs is decided for you, and reported afterwards:

| Situation | What happens |
| --- | --- |
| A PHP service is identified and its container is running | `compose exec -T <service> composer dump-autoload`. An autoloader is read by the PHP that uses it, so the container's own Composer writes it. |
| A PHP service is known but the container is down | Composer runs on this computer, and the result says so. |
| No PHP service could be identified, or the project has no Compose file | Composer runs on this computer. For a native project that is simply where it belongs, so nothing is explained. |

On the host, `composer` is resolved on `PATH` - including the `.bat` shim on Windows,
which is handed to `cmd` as an argument rather than pasted into a command line - and a
`composer.phar` in the project is the fallback. Composer's own output is kept verbatim
in a collapsible block, bounded to its last twenty lines.

This is the only Composer command SymfoLinker runs. `dump-autoload` writes only inside
`vendor/composer/` and installs nothing; `composer install` and `composer update` stay
out of this app, because repairing a situation by reinstalling is exactly what hard
rule 2.4 forbids. The development-root lock is held while it runs, so a switch cannot
move directories under it.

### Link kind

Links are created as **relative symlinks**, so the same link resolves on the host and inside a container that mounts the tree at a different prefix (plan section 24):

```text
app/vendor/acme/bundle -> ../../../bundle
```

Windows only permits symlink creation under Developer Mode or elevation. Rather than leaving the core feature unusable there, SymfoLinker falls back to a **directory junction**, which needs no privileges. A junction stores an absolute target, so container path mapping cannot be assumed to hold:

```text
app\vendor\acme\bundle => D:\dev\bundle
```

`activate_local` reports which kind it created, so the difference is visible rather than silent. Linux and macOS always get a relative symlink. Enabling Developer Mode on Windows yields one too, without any change to SymfoLinker.

### Container engine

SymfoLinker talks to **Docker or Podman**, whichever answers first. Podman ships a Compose-compatible CLI, so every subcommand used here takes identical arguments; only the binary name differs:

```text
docker compose ps|config|exec      podman compose ps|config|exec
```

Detection caches the selected engine. Failed detection expires after 30 seconds, and an explicit workspace Refresh clears detection immediately. Starting Docker or Podman therefore does not require an app restart. Docker is preferred when both answer. Probes have a five-second timeout and Compose commands a twenty-second timeout; collecting output is also bounded if a subprocess inherits the output pipes.

### Container status

Status comes from `docker compose ps --all` in the project directory. The daemon is probed separately, so an unreachable Docker never reads as a Compose error.

| Condition | Shown as |
| --- | --- |
| All services running | `<running>/<total> services running`, green |
| Some services stopped | `<running>/<total> services running`, amber |
| Compose file present, no containers created | `No containers` |
| Docker daemon unreachable or CLI missing | `Docker unavailable`, red, with the reason |
| No Compose file | `Native` |

Per service, the Runtime panel lists the Compose service name, container name, state, health and published ports. State and health labels are translated; status text from the Docker CLI is shown verbatim.

The status refreshes every five minutes while the project stays selected and the window is visible, and immediately on **Sync**. Polling pauses when the window is hidden, because each refresh starts a Git and a Docker process.

### Container inspection

Runtime shows live container state on every refresh; **Inspect container** additionally reads mounts and validates the links inside the container. It is a separate action on purpose: it runs `compose config` plus one `exec` per linked package, which is far too expensive to poll. The same inspection runs automatically after a switch, as part of [synchronising the mounts](#composer-reinstall-and-missing-container-mounts).

| Step | What it does |
| --- | --- |
| Service selection | Suggests the PHP service by name (`php-fpm`, `php`, `app`, `backend`, `web`, or a name containing one). Choosing one stores it in `<root>/.symfolinker/config.json`; it suggests nothing rather than guessing when no name matches. |
| Mount inspection | Reads bind mounts from `compose config`. Named volumes and tmpfs are dropped: they have no host side. |
| Path mapping | Translates a host path to the container path, longest matching mount first. Separator- and case-insensitive, so Windows paths map too. |
| Link validation | Per locally linked package: `exec -T <service> test -e <path>`, then `readlink <path>`. Arguments are passed directly, never through a shell. |

Without a Compose file the runtime is native and only host validation applies. A project's own Compose files are only ever read; the mounts a switch requires go into a generated override above the projects.

### Warnings

Warnings are collected per scan and displayed in an expandable block above the panel content. They cover unreadable directories in the root, invalid manifests, invalid Composer package names and duplicate package names.

### More than one checkout

The same repository is often checked out twice under one root on purpose: one clone to
test a multi-tenant setup, another on a QA branch so neither has to be stashed for the
other. Both declare the same Composer name, so the scanner cannot tell which one a link
should use, and it refuses to guess - linking the wrong clone silently would be worse
than linking nothing at all.

Instead the candidates are listed under the scan warnings, each with its branch and
commit, which is what actually distinguishes two clones of one repository. Choosing one
stores it in `<root>/.symfolinker/config.json` and the warning stops.

The choice belongs to the development root, not to a single project: every project in
the root that requires that package links the chosen checkout. *Ask again* clears it. A
stored choice naming a checkout that has since been removed or renamed is ignored rather
than quietly redirected, so the question simply reappears.

### File locking

Health reports whether this filesystem can lock files. It usually can; a
`\\wsl.localhost` development root and some network shares cannot. Switching still
works there, because within one running copy of SymfoLinker the lock is this app's own
discipline, but a second copy working in the same root could not be noticed. The scan
warnings say so when that is the case.

If an operation is interrupted badly enough that the lock survives it, the error offers
*Take over the lock*. It is never automatic, and it still refuses while a live holder
has the file open - which is the operating system's judgement rather than a guess.

## Common messages

| English message | Cause and solution |
| --- | --- |
| *Open the desktop app with npm run desktop…* | You are using the browser version. Real scanning requires the Tauri shell. |
| *The development root does not exist or is not readable.* | The path is missing or lacks read permissions. |
| *Choose a directory as the development root.* | The supplied path is a file. |
| *No Composer projects found* | You may be inside a project. Choose its parent directory. |
| *…: multiple local projects found* | Two checkouts share a Composer name. Choose which one to link under the scan warnings; see [More than one checkout](#more-than-one-checkout). |
| *Another SymfoLinker instance is working in this development root…* | A second copy of the app holds the lock. Close it, or wait for it to finish. |
| *SymfoLinker is still finishing another operation in this development root…* | This same copy is mid-operation. It queues briefly by itself; a long `composer dump-autoload` outlasts that wait. |
| *SymfoLinker locked this development root … and never finished* | An interrupted run left the lock behind. Use *Take over the lock* on the error. |
| *This filesystem cannot lock files…* | A `\\wsl.localhost` root or a network share. Switching works; a second instance cannot be detected. |
| *Invalid path* | A vendor path or ancestor is a link or is not a directory. |
| *No Git information* | The project has no `.git`, or Git is missing from `PATH`. |
| *{engine} is unavailable. Is it running?* | The Docker or Podman daemon did not respond. Container status is unknown until it does. |
| *No container engine found. Is docker or podman in PATH?* | Neither executable is on the `PATH` of the app process. |
| *No containers have been created for this Compose project.* | A Compose file exists, but `docker compose up` has not run yet. |
