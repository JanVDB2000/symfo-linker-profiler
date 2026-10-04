# SymfoLinker

[![CI](https://github.com/JanVDB2000/symfo-linker-profiler/actions/workflows/ci.yml/badge.svg)](https://github.com/JanVDB2000/symfo-linker-profiler/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

> **Safely switch between the original Composer vendor version and a local symlink, without touching project configuration.**

SymfoLinker is a local desktop app for PHP developers who work on a package and the application that consumes it at the same time. It scans the projects under one development root, finds dependencies that also exist as a local checkout, and switches them between the Composer version and that checkout, without editing `composer.json` and without ever deleting the original package.

**Contents:** [Download](#download) · [The problem](#the-problem) · [What it does](#what-it-does) · [Safety model](#safety-model) · [Getting started](#getting-started) · [Development root](#the-development-root) · [Interface](#the-interface) · [Languages](#languages) · [Scan rules](#scan-rules) · [Status values](#status-values) · [Data contract](#data-contract-ipc) · [Project structure](#project-structure) · [Verification](#verification) · [Messages](#common-messages) · [Roadmap](#roadmap) · [License](#license)

## Download

Most people do not need this repository. Grab an installer from the [latest release](https://github.com/JanVDB2000/symfo-linker-profiler/releases/latest) instead:

| Platform | File |
| --- | --- |
| Windows | `.msi` or `.exe` setup |
| macOS | `.dmg` (separate builds for Apple silicon and Intel) |
| Linux | `.AppImage` (runs anywhere, larger because it carries its own webview), `.deb` (Debian/Ubuntu) or `.rpm` (Fedora/RHEL) |

Releases also contain `SymfoLinker_*.app.tar.gz`. Those are macOS update bundles, not downloads: use the `.dmg`.

The rest of this README is for building from source, which you only need if you want to modify SymfoLinker.

### These builds are not signed

Code signing certificates cost money per year, so the releases are unsigned. Your operating system will say so, and that warning is accurate: it means nobody has paid to vouch for the binary, not that the binary has been checked and found bad.

- **Windows** shows "Windows protected your PC". Choose *More info*, then *Run anyway*.
- **macOS** says the app "cannot be opened because the developer cannot be verified". Right-click the app, choose *Open*, then confirm; or run `xattr -dr com.apple.quarantine /Applications/SymfoLinker.app`.
- **Linux** AppImages need the executable bit: `chmod +x SymfoLinker_*.AppImage`.

If you would rather not trust an unsigned binary, building from source is the alternative, and every release is built from this repository by [the release workflow](.github/workflows/release.yml).

## The problem

You develop a bundle and an application that uses it through Composer. During development, you want the application to use your local working version while preserving the version installed by Composer.

```text
VENDOR                                      LOCAL
app/vendor/acme/bundle   (Composer)   ⇄     app/vendor/acme/bundle -> ../../../bundle
```

Switching manually means using `rm -rf` inside `vendor/`, running `composer install` again or temporarily changing `composer.json`. All of these can lose work. SymfoLinker makes the switch explicit, reversible and visible.

## What it does

Every milestone in [the implementation plan](docs/SymfoLinker-implementation-plan.md) is implemented. Built with Tauri 2, Vue 3, TypeScript, Pinia and Rust; version `0.2.0`.

**Discover**

- Scan the immediate subdirectories of a development root for `composer.json`, and map `require` and `require-dev` onto the local projects that provide them.
- Show per project: Git branch, short commit, changed files, vendor and link status, backup status and resolved paths.
- Warn about invalid manifests, duplicate package names and unreadable directories.

**Switch**

- Move a package between the Composer version and a local checkout with a `VENDOR` / `LOCAL` toggle, behind a confirmation dialog naming the backup location and the local source.
- Preserve the original Composer directory in `vendor/.symfolinker/`, under a write guard and an exclusive lock on the development root, rolling back automatically when a step fails.
- Link with a relative symlink, falling back to a junction on Windows without Developer Mode.

**Validate**

- Refresh Git and container status for the selected project, on request or every five minutes while the window is visible.
- Inspect the container on request: read Compose services and bind mounts, pick or auto-suggest the PHP service, map host paths to container paths, and check each linked package inside the container.
- Work with either Docker or Podman, whichever answers first.
- Show, across all projects, which local package is linked where and on which branch.
- Switch the interface between English, Dutch, French and German.

### Limits worth knowing

- Backups from older versions without a journal are marked **Unrecognized**. They can be restored explicitly, but their provenance cannot be verified.
- The health check reports readability and path status, not write permissions.
- A recovery journal records the project, package, installed version (when known), timestamp and operation state. Filesystem inspection remains the source of truth for active modes; journal metadata is not a content checksum.

### What it writes, and where

Projects are read-only outside `vendor/`. SymfoLinker writes in exactly two places:

| Location | Contents |
| --- | --- |
| `<project>/vendor/` | The package link, preserved Composer directory, recovery journal (`vendor/.symfolinker/state.json`) and temporarily staged restore links. |
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

If `composer install` replaces a local link with a fresh vendor directory while an earlier backup remains, switching back to LOCAL now archives the earlier directory under `vendor/.symfolinker/.history/<vendor>/<package>/`. The newly installed Composer directory becomes the active backup. Switching to VENDOR restores that newest version. Both versions are preserved; a failed link step rolls the directory moves back. Each archived directory has a JSON sidecar with its recorded package/project/version metadata. Archives are never automatically deleted.

In **Runtime**, container inspection reads the running service's actual bind mounts. An application-only mount such as `/home/user/dev/app:/application` does not expose sibling repositories: `vendor/acme/bundle -> ../../../bundle` resolves to `/bundle` inside the container. **Missing local source mounts** lists the required host sources and exact container destinations, including mounts that can be prepared before activating LOCAL.

**Apply mounts and recreate PHP service** opens a confirmation with those paths and the service that will briefly stop. SymfoLinker verifies the running container's project, service, original Compose files and custom environment files from its labels, writes an additional JSON Compose override above the projects, validates the combined configuration, and runs `compose up -d --no-deps --no-build --pull never --force-recreate` for that service only. Project Compose files remain unchanged. A mounted path already occupied by another source is refused.

Docker cannot attach an extra bind mount to an already-running container in place; a normal restart does not apply new mounts. The service must be recreated. Its image and mounted volumes are reused, but data stored only in the container's writable layer is not preserved by recreation. If the original Compose context cannot be verified (for example, a Podman provider without compatible labels), the required paths are displayed but automatic recreation is disabled. Later external `docker compose up` commands must include the generated override to retain these additional mounts; applying mounts again in SymfoLinker restores them if they were omitted.

## Safety model

The hard rules from the implementation plan guide every milestone. The scanner enforces read-only inspection and path boundaries. The mutation safeguards are now implemented as well: every write passes `WriteGuard`, mutations require an exclusive lock on the development root, and a failed step rolls the backup back.

| Rule | Meaning |
| --- | --- |
| **Read-only outside `vendor/`** | Read `composer.json`, `composer.lock`, Git state and Docker configuration; never write them. |
| **App state above the projects** | Settings and locks belong in `<root>/.symfolinker/`. Package recovery metadata stays beside its backup inside `vendor/.symfolinker/`. |
| **Always preserve the original vendor package** | During a swap, move the Composer directory to `vendor/.symfolinker/<vendor>/<package>` and keep it there while the local version is active. |
| **No destructive fallback** | Never automatically use `rm -rf`, `composer install/update`, `git reset --hard`, `git clean` or `git stash` to "fix" an error. Stop with a message when safety cannot be established. |
| **Atomicity** | Every mutation must support rollback: if link creation fails after making a backup, restore the backup to its original vendor location. |

`WriteGuard::assert_vendor_path` is the single gate: it allows a target only strictly inside `<project>/vendor/`, rejects the `vendor/` directory itself, rejects traversal components, and refuses when any directory between the project and the target is a link. Mutating functions take the lock as a parameter, so holding it is a compile-time requirement rather than a convention. `rename` is used instead of copy and delete, so a package never exists in neither place.

Paths from projects are untrusted input. The scanner canonicalizes the root, ignores directories that are themselves links, and refuses package inspection when an intermediate directory between the project and target is a link or is not a directory. On Windows, junctions (reparse points) are also recognized as links.

## Getting started

### Requirements

| Component | Requirement |
| --- | --- |
| Node.js | 22.12+ (or 20.19+) with npm; translation tests require Node.js 22.12+ |
| Rust | 1.87+, installed through [rustup](https://rustup.rs). Not optional: the backend is Rust, so `npm run build` alone will not produce a working app. See [Version pinning](#version-pinning). |
| Git | Required for branch and status information; otherwise that information is unavailable |
| Linux | WebKitGTK and friends, see below |
| macOS | Xcode Command Line Tools (`xcode-select --install`) |
| Windows | MSVC C++ build tools and WebView2 |

Tauri renders through the operating system's own webview, so the desktop build needs that webview's development headers. Everything below is a one-time setup per machine.

**Linux (Debian/Ubuntu)**

```bash
sudo apt update
sudo apt install build-essential libwebkit2gtk-4.1-dev libxdo-dev   libssl-dev libayatana-appindicator3-dev librsvg2-dev

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

That last line matters: `rustup` adds `cargo` to your `PATH` only for new shells, so without it the terminal you are in still cannot find `cargo`. Add `patchelf` as well if you enable bundling later.

For Fedora, Arch and other distributions, see the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

### Install and run

```powershell
npm install
npm run desktop
```

`npm run desktop` starts Vite at `127.0.0.1:1420` and builds the Tauri shell with the `desktop` feature. `npm run dev` runs only the frontend in the browser, where development fixtures demonstrate the interface. Use `?empty` for the initial empty state and `?error` for a simulated scan failure. Production browser builds never present fixture data as real scans; local scanning requires the desktop app.

### Build troubleshooting

The messages below are the ones that actually come up on a fresh machine. The first two are real failures; the last two are noise that can be ignored.

| What you see | What it means |
| --- | --- |
| `failed to run \`cargo metadata\` ... No such file or directory (os error 2)` | Rust is not installed, or `cargo` is not on the `PATH` of this shell. The path is missing, not the file. Install rustup, then run `source "$HOME/.cargo/env"` or open a new terminal. |
| `failed to run custom build command for 'glib-sys'`, or pkg-config errors naming `webkit2gtk`, `javascriptcoregtk` or `soup` | The Linux system libraries above are missing. Install them and build again. |
| `npm warn install-scripts ... esbuild@0.28.2 (postinstall: node install.js)` | Harmless. esbuild ships its binary as a platform-specific optional dependency, so the postinstall has nothing left to do and npm's script approval is not needed. Approving it is not recommended for a dependency that does not need it. |
| `WARNING: Glycin running without additional sandbox inside snap confinement.` | Harmless, and not from this app. It appears when the terminal itself runs inside a snap (a snap-installed IDE, for example) and an image library notices it cannot nest its sandbox. Running from a normal terminal makes it disappear. |

### Version pinning

The Tauri npm packages and Rust crates must share a major/minor version, or `tauri build` stops before compiling.

`.cargo/config.toml` sets `resolver.incompatible-rust-versions = "fallback"`, so Cargo resolves against the `rust-version = "1.87"` declared in `src-tauri/Cargo.toml` instead of taking the newest crate. That holds the Tauri crates at 2.11.x, because 2.12 requires Rust 1.90. The npm packages are therefore pinned to match:

```json
"@tauri-apps/api": "~2.11.1",
"@tauri-apps/plugin-dialog": "~2.7.3"
```

Moving to Tauri 2.12 is a single coordinated change: install Rust 1.90 (`rustup update stable`), raise `rust-version`, run `cargo update`, and lift both npm pins together. Changing only one side reintroduces the mismatch.

### npm scripts

| Script | Purpose |
| --- | --- |
| `npm run desktop` | Desktop app in development (`tauri dev --features desktop`). |
| `npm run desktop:build` | Build the desktop application with the `desktop` feature, including installers. |
| `npm run desktop` / `desktop:build` / `check:native` | Each runs `scripts/check-prerequisites.mjs` first, so a missing Rust toolchain or missing WebKitGTK headers produce a readable message instead of a `cargo metadata` error. |
| `npm run tauri` | Raw Tauri CLI passthrough. Note that `npm run tauri build` leaves out the `desktop` feature, so Cargo matches no binary target and no executable is produced; use `npm run desktop:build`. |
| `npm run dev` | Frontend on port 1420 (`strictPort`). |
| `npm run build` | Typecheck with `vue-tsc --noEmit`, then build production assets into `dist/`. |
| `npm run preview` | Serve the built frontend. |
| `npm test` | Rust tests without desktop features (`--no-default-features`). |
| `npm run test:i18n` | Translation completeness, interpolation, fallback, language switching and persistence tests. |
| `npm run test:cache` | Scan cache tests: restoring after a restart, root mismatches, rejected entries and clearing. |
| `npm run check:native` | `cargo check` with the `desktop` feature. |
| `npm run format:check` | `cargo fmt -- --check`. |

### App icon

The icon is a neutral swap mark, deliberately carrying no Symfony branding (see [Trademarks](#trademarks)). The square source is `app-icon.png` (1024×1024, transparent). Regenerate the complete set after a change:

```powershell
npx tauri icon app-icon.png
magick app-icon.png -resize 256x256 public/icon.png
```

`npx tauri icon` populates `src-tauri/icons/` with `icon.ico`, `icon.icns`, PNG sizes and Windows Store, iOS and Android variants. `src-tauri/tauri.conf.json` references these through `bundle.icon`. `public/icon.png` is the favicon and header logo.

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
| **Projects** | Composer package, path, branch, commit, working directory and container status, with a freshness badge and a Sync button; all locally available dependencies with mode, branch, host status, backup status and expandable paths. The Mode column is a `VENDOR` / `LOCAL` toggle; switching asks for confirmation first. |
| **Active links** | One block per local package: its branch and changed-file count, the local source path, every project linking to it, and its onward chain. Package and project names navigate. |
| **Runtime** | Docker Compose or native runtime, detected Compose file, host path and live container status. |
| **Health** | Root readability, project count, Git information availability and package issues. |
| **Settings** | Language, theme, scan scope and project access. |

On first launch, the theme follows the system preference (`prefers-color-scheme`), including changes during use. The switch below the menu toggles light and dark; **Settings → Appearance** also offers Automatic. Next to the theme switch, you can select normal or full width. Both choices are saved locally. The layout shrinks below 1200 px, and below 760 px the collector menu becomes horizontal.

## Languages

Open **Settings → Appearance → Language** and select **English (EN)**, **Nederlands (NL)**, **Français (FR)** or **Deutsch (DE)**. English is the default. Changes apply immediately without restarting or rescanning and are saved as `symfolinker.language`. The document's `lang` attribute and time formatting follow the selected language.

All app-owned labels, tooltips, empty states, status messages, errors and warnings use translation files:

```text
src/i18n/locales/en.json
src/i18n/locales/nl.json
src/i18n/locales/fr.json
src/i18n/locales/de.json
```

English source text is the translation key. Call `t(key, params)` for UI text and `translateMessage(message)` for backend messages. Each catalog must contain the same keys and placeholders, such as `{count}`, `{path}` and `{name}`. Missing entries fall back to English. Parameters are inserted in one pass and displayed as text.

Rust sends language-independent `{ key, params }` objects so existing errors and warnings also change language immediately. Package names, paths, Git branches and external Docker output remain source data. Docker's state and health labels are translated, while its CLI status/detail text is preserved verbatim.

The README and implementation plan are maintained in English.

## Scan rules

The scan is deliberately strict and predictable:

- Inspect **only immediate subdirectories** of the root; do not recurse.
- Skip names beginning with a dot.
- Do not treat a symlink or junction directory as a project.
- The canonical path must stay inside the root; otherwise skip it.
- A directory without `composer.json` is not a project.
- Skip unreadable or invalid JSON manifests **with** a warning.
- Package names must have the form `vendor/package`, using lowercase letters, digits and `_ . -`, starting with an alphanumeric character and not ending with a dot. Platform requirements (`php`, `ext-json`) and traversal attempts (`../bundle`, `acme/..`, `C:/bundle`) are excluded from mapping.
- If multiple projects share a package name, skip the mapping with a warning rather than guessing.
- Do not map a project to itself.
- **Multiple and chained links** are supported. Each project is inspected at its own path: `a` can link both `acme/b` and `acme/d`, while `d` links `acme/c`. Links do not interfere because the scanner never traverses a package link to inspect another project.
- Sort projects by directory name and packages by package name.
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

### Link kind

Links are created as **relative symlinks**, so the same link resolves on the host and inside a container that mounts the tree at a different prefix (plan section 24):

```text
app/vendor/acme/bundle -> ../../../bundle
```

Windows only permits symlink creation under Developer Mode or elevation. Rather than leaving the core feature unusable there, SymfoLinker falls back to a **directory junction**, which needs no privileges. A junction stores an absolute target, so container path mapping cannot be assumed to hold:

```text
appendorcmeundle => D:\devundle
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

Runtime shows live container state on every refresh; **Inspect container** additionally reads mounts and validates the links inside the container. It is a separate action on purpose: it runs `compose config` plus one `exec` per linked package, which is far too expensive to poll.

| Step | What it does |
| --- | --- |
| Service selection | Suggests the PHP service by name (`php-fpm`, `php`, `app`, `backend`, `web`, or a name containing one). Choosing one stores it in `<root>/.symfolinker/config.json`; it suggests nothing rather than guessing when no name matches. |
| Mount inspection | Reads bind mounts from `compose config`. Named volumes and tmpfs are dropped: they have no host side. |
| Path mapping | Translates a host path to the container path, longest matching mount first. Separator- and case-insensitive, so Windows paths map too. |
| Link validation | Per locally linked package: `exec -T <service> test -e <path>`, then `readlink <path>`. Arguments are passed directly, never through a shell. |

Without a Compose file the runtime is native and only host validation applies. Container configuration is only ever read.

### Warnings

Warnings are collected per scan and displayed in an expandable block above the panel content. They cover unreadable directories in the root, invalid manifests, invalid Composer package names and duplicate package names.

## Data contract (IPC)

The frontend communicates with Rust through Tauri commands:

```ts
invoke<ScanResult>('scan_projects', { developmentRoot: string })
invoke<ProjectStatus>('project_status', { projectPath: string })
invoke<ScanResult>('activate_local',  { developmentRoot: string, projectId: string, packageName: string })
invoke<ScanResult>('activate_vendor', { developmentRoot: string, projectId: string, packageName: string })
invoke<ContainerReport>('inspect_container', { developmentRoot: string, projectId: string })
invoke<void>('set_php_service', { developmentRoot: string, projectPath: string, service: string | null })
```

The two switch commands take **ids, never paths**. They resolve the project and its local source by rescanning the development root, so the webview cannot name an arbitrary directory for a mutation. Both return the scan taken after the switch, which is what the interface renders: there is no optimistic state for a filesystem change (plan section 48).

Scanning and status inspection run on blocking worker threads so the webview stays responsive. Rust serializes domain fields in `camelCase`. TypeScript counterparts are defined in [`src/types/index.ts`](src/types/index.ts).

Errors, scan warnings and optional Docker messages use this envelope:

```json
{
  "key": "{name}: composer.json is invalid or unreadable; project skipped.",
  "params": { "name": "my-project" }
}
```

The frontend translates the key at render time and inserts the parameters. Raw operating system errors are not exposed. External Docker diagnostic text is included as a parameter in a translated error message.

The webview is restricted by the CSP in `src-tauri/tauri.conf.json`. The capability in `src-tauri/capabilities/default.json` allows `core:default` and `dialog:allow-open`, providing the directory picker.

## Project structure

```text
app-icon.png                    # square source icon (1024²) for npx tauri icon
public/icon.png                 # favicon and header logo
public/fonts/                   # bundled JetBrains Mono and its license
public/SYMFONY-LICENSE.txt      # license for the profiler styling this interface follows
src/
  App.vue                       # profiler layout with five panels
  components/ProfilerIcon.vue   # inline SVG icons for menus, labels and tables
  components/Spinner.vue        # rotating busy indicator for scans
  dev/fixture.ts                # demo workspace for npm run dev; excluded from production builds
  i18n/index.ts                 # reactive language setting and message translation
  i18n/translator.ts            # locale resolution and placeholder interpolation
  i18n/locales/                 # EN, NL, FR and DE translation catalogs
  main.ts                       # Vue + Pinia bootstrap
  stores/workspace.ts           # scan state, filters, links, chains and polling
  stores/scanCache.ts           # validated localStorage copy of the last scan
  types/index.ts                # TypeScript counterparts of Rust models
  style.css                     # profiler styling with dark and light themes
src-tauri/
  icons/                        # generated app icons (.ico, .icns, PNG, Store, iOS, Android)
  src/lib.rs                    # Tauri builder and IPC commands
  src/main.rs                   # desktop binary (requires desktop feature)
  src/discovery.rs              # scanning, path safety and link/backup/Git inspection
  src/write_guard.rs            # the only gate for mutating writes; vendor/ is the sole writable zone
  src/lock.rs                   # exclusive lock per development root, in <root>/.symfolinker/lock
  src/backup.rs                 # vendor <-> backup moves with rollback
  src/links.rs                  # relative symlink creation, junction fallback and link inspection
  src/swap.rs                   # activate_local and activate_vendor
  src/activate.rs               # id resolution, locking and the rescan returned to the interface
  src/engine.rs                 # Docker or Podman detection, cached per process
  src/compose.rs                # compose config parsing, service suggestion, path mapping
  src/container.rs              # mount inspection and in-container link validation
  src/config.rs                 # <root>/.symfolinker/config.json, the PHP service per project
  src/errors.rs                 # explicit failure reasons, rendered through the message envelope
  src/runtime.rs                # Git and Docker runtime status
  src/models.rs                 # domain models and translation message envelope
  tests/scanner.rs              # temporary-workspace integration tests
  tests/runtime.rs              # Compose output parsing tests
  tests/messages.rs             # translation message envelope tests
  tests/write_guard.rs          # unit tests for the write boundary
  tests/backup.rs               # temporary-workspace tests for moves, rollback and locking
  tests/swap.rs                 # relative paths, round trips, broken links and wrong targets
  tests/activate.rs             # id resolution, refusals and state read back after a switch
  tests/engine.rs               # engine preference and fallback
  tests/compose.rs              # config parsing, service suggestion, path mapping and config.json
  capabilities/default.json
  tauri.conf.json
tests/i18n.test.mjs              # translation and language setting tests
tests/scan-cache.test.mjs        # restoring, rejecting and clearing the stored scan
.cargo/config.toml              # resolver fallback to Rust 1.87-compatible crate versions
docs/SymfoLinker-implementation-plan.md
```

File locking uses the `fs4` crate rather than a hand-rolled lock file: advisory locking differs per platform and a stale lock after a crash is exactly the bug a custom implementation tends to ship. It is the only non-Tauri runtime dependency besides Serde.

The Rust crate is split into a library (`symfolinker_lib`) and a binary. Tauri is behind the optional `desktop` feature, so discovery can be tested without webview dependencies.

## Verification

```powershell
npm run build
npm test
npm run test:i18n
npm run test:cache
npm run format:check
npm run check:native
```

Scanner tests create and clean up temporary workspaces. They cover discovery, mapping, read-only behavior, invalid and duplicate manifests, rejected package names, backup and link status, ancestor escapes, multiple and chained links (`a → b`, `a → d`, `d → c`) and Git status. Windows junction tests do not require elevated permissions. Three additional real-symlink tests are ignored on Windows by default because they require Developer Mode or symlink privileges:

```powershell
npm test -- -- --include-ignored
```

Engine tests cover preferring Docker, falling back to Podman, skipping a binary that exists but does not answer, and reporting nothing when neither does. Compose tests cover config parsing with named volumes dropped, service suggestion order and its refusal to guess, path mapping including the mount root, the most specific mount, paths outside every mount, Windows separators and casing, and the config file round trip including a corrupt file reading as empty.

Activate tests cover resolving a project and package by id, refusing an unknown project id and a package the project does not require (leaving the vendor directory untouched in both cases), the state read back after a switch in each direction, and that the development-root lock is released so a second switch can follow.

Swap tests cover the relative path calculation, a vendor-to-local switch that serves the local files while the Composer directory stays preserved, the way back, a repeated round trip, a missing local project that changes nothing, a refusal to delete a real directory, an unexpected link target, a broken link, and recovery from a broken link without the local project. On Windows without symlink privileges they exercise the junction fallback, which is how they run on a default Windows install; the symlink branch is covered on Linux, macOS, and Windows with Developer Mode enabled.

Backup tests run against real temporary directories and cover the move and its restore, a refused overwrite of an existing backup, a missing vendor package, a missing backup, a vendor path recreated by `composer install`, rollback after a failing step, and a genuine I/O failure (on Windows a file held open inside the package, on Unix a read-only parent). They assert the package contents survive and that a refusal leaves both sides untouched. Write guard tests cover the allowed vendor and backup paths against `composer.json`, `src/`, `.git/`, the `vendor/` directory itself, traversal, another project, and a path behind a non-directory. Lock tests cover refusal of a second lock, release on drop, and that no `.symfolinker` directory appears in a project root.

Compose parsing tests cover JSON-lines and JSON-array output, stopped services, missing service names, unpublished and duplicate ports, and unparsable lines; because every field has a default, they also assert that unrelated JSON never becomes an empty phantom service. Message tests cover the key/params envelope Rust sends for errors and warnings.

Translation tests check catalog keys and placeholders, interpolation of dynamic data, English fallback, persisted language selection and immediate changes to rendered UI messages.

Cache and interaction tests cover late status/container responses after project changes, failed service saves, uncertain swap readback, workspace shortcuts and profile persistence. Scan cache tests reopen the store against a stubbed storage: a restored scan keeps its timestamp and selected project, a selection that disappeared falls back to the first project, a cache from another development root is ignored, unreadable, incomplete and outdated entries are discarded instead of rendered, and clearing empties both the interface and the stored copy.

Reliability tests cover missing backups without removing links, exact link rollback after an I/O failure (Windows), interrupted-operation recovery without a local checkout, profile preflight and rollback, corrupt journals, and locked configuration writes. Process tests cover both output streams, missing binaries, hung commands and descendants that keep output pipes open.

## Common messages

| English message | Cause and solution |
| --- | --- |
| *Open the desktop app with npm run desktop…* | You are using the browser version. Real scanning requires the Tauri shell. |
| *The development root does not exist or is not readable.* | The path is missing or lacks read permissions. |
| *Choose a directory as the development root.* | The supplied path is a file. |
| *No Composer projects found* | You may be inside a project. Choose its parent directory. |
| *…: multiple local projects found* | Two projects share a Composer name; make the names unique. |
| *Invalid path* | A vendor path or ancestor is a link or is not a directory. |
| *No Git information* | The project has no `.git`, or Git is missing from `PATH`. |
| *{engine} is unavailable. Is it running?* | The Docker or Podman daemon did not respond. Container status is unknown until it does. |
| *No container engine found. Is docker or podman in PATH?* | Neither executable is on the `PATH` of the app process. |
| *No containers have been created for this Compose project.* | A Compose file exists, but `docker compose up` has not run yet. |

## Roadmap

Every milestone in this plan is implemented. Recovery metadata, bounded external commands, saved workspaces and link profiles are now implemented as well. Released installers remain unsigned.

| # | Milestone | Contents |
| --- | --- | --- |
| 1 | **Read-only scanner** ✓ | Discovery, mapping, Git and vendor status. No mutations. |
| 2 | **Safe backup engine** ✓ | Backend `vendor → backup` and `backup → vendor`, with WriteGuard, development-root locking and rollback. No symlinks and no GUI action yet. |
| 3 | **Local symlink** ✓ | Relative symlinks, junction fallback on Windows, link inspection, broken links and unexpected targets. No GUI action yet. |
| 4 | **GUI switch** ✓ | `VENDOR` / `LOCAL` toggle per package with confirmation dialogs. The interface updates only after the backend returns a fresh scan. |
| 5 | **Docker and Podman** ✓ | Compose discovery, PHP service selection, mount inspection, host/container path mapping and container link validation. Container configuration stays read-only. |
| 6 | **Links dashboard** ✓ | One block per local package: its branch, its working-directory state, every project linking to it, and its onward chain. |

### Definition of Done for v0.1

Projects are discovered automatically; dependencies map to local repositories; branch and dirty status are visible; local/vendor mode is detected correctly; the original vendor directory is always preserved; local symlinks are relative; restoration works without Composer; failed swaps roll back automatically; no writes outside `vendor/` are possible within projects; app state lives only above the projects; Docker/PHP-FPM validation works for normal bind mounts; users can switch through the GUI; active links across all projects are visible.

## Releasing

Installers are built by [`.github/workflows/release.yml`](.github/workflows/release.yml), never by hand: each platform has to build on its own runner because an installer embeds that operating system's webview and native libraries.

```bash
git tag v0.2.0
git push origin v0.2.0
```

That builds Windows, Linux, and both macOS architectures, then opens a **draft** release with the artifacts attached, so it can be checked before anyone can download it. Publish the draft to make it live.

Bump `version` in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` to match the tag before tagging.

## Contributing

Issues and pull requests are welcome. Before opening a pull request, run the checks in [Verification](#verification); CI runs the frontend build, translation/cache/interaction tests, Rust tests, formatting and Clippy on Linux, macOS and Windows, and checks the desktop feature on Linux.

Two conventions worth knowing:

- Every mutating filesystem call goes through `WriteGuard` and takes the development-root lock as a parameter, so neither can be forgotten. Do not bypass them.
- The hard rules in [the implementation plan](docs/SymfoLinker-implementation-plan.md) (sections 2.1 to 2.5) are not style preferences; they are what keeps a failed swap from losing someone's work.

## License

[MIT](LICENSE).

Bundled third-party assets keep their own licenses: JetBrains Mono in [`public/fonts/LICENSE.txt`](public/fonts/LICENSE.txt), and the Symfony profiler styling this interface follows in [`public/SYMFONY-LICENSE.txt`](public/SYMFONY-LICENSE.txt).

### Trademarks

SymfoLinker is an independent project. It is **not affiliated with, endorsed by, or sponsored by Symfony SAS**.

Symfony is a trademark of Symfony SAS. The interface deliberately mirrors the look of the Symfony WebProfilerBundle, whose code is MIT licensed, and the name refers to Symfony only to describe what this tool works with. The [Symfony trademark policy](https://symfony.com/trademark) covers the name and logo separately from that MIT license, so no Symfony logo or brand mark is used anywhere in this application or its icons.

## Technical references

- [Tauri 2: Vite configuration](https://v2.tauri.app/start/frontend/vite/)
- [Tauri 2: platform prerequisites](https://v2.tauri.app/start/prerequisites/)
- [Vue 3: TypeScript guidelines](https://vuejs.org/guide/typescript/overview)
- [Composer: composer.json schema](https://getcomposer.org/doc/04-schema.md)
- [Symfony: Profiler and WebProfilerBundle](https://symfony.com/doc/current/profiler.html)
- [docs/SymfoLinker-implementation-plan.md](docs/SymfoLinker-implementation-plan.md): full specification with hard rules, error types and test cases.
