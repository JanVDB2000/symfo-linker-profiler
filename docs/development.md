# Development guide

[Back to the README](../README.md) | [User guide](usage.md)

SymfoLinker uses Tauri 2, Vue 3, TypeScript, Pinia and Rust.

## Contents

- [Getting started](#getting-started)
- [Safety model](#safety-model)
- [Data contract (IPC)](#data-contract-ipc)
- [Translations](#translations)
- [Project structure](#project-structure)
- [Verification](#verification)
- [Releasing](#releasing)
- [Contributing](#contributing)
- [Trademarks](#trademarks)
- [Technical references](#technical-references)
- [Roadmap](roadmap.md)

## Getting started

### Requirements

| Component | Requirement |
| --- | --- |
| Node.js | 22.12+ (or 20.19+) with npm; translation tests require Node.js 22.12+ |
| Rust | 1.87+, installed through [rustup](https://rustup.rs). Not optional: the backend is Rust, so `npm run build` alone will not produce a working app. See [Version pinning](#version-pinning). |
| Git | Required for branch and status information; otherwise that information is unavailable |
| Linux | WebKitGTK and friends, see below |
| NixOS | Nothing: `nix develop` provides the toolchain and the libraries together |
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

**NixOS**

```bash
nix develop
npm ci
npm run desktop
```

Nothing is installed: the flake carries Rust, Node, WebKitGTK and GTK, so the shell has
them and the system does not. The usual advice does not transfer here - there is no
`apt`, and the rustup tarball's dynamically linked binaries do not run against a
non-FHS loader - which is why `scripts/check-prerequisites.mjs` recognises NixOS and
answers `nix develop` instead of naming packages.

`nix build .#default` produces the installable package, and CI builds it on every push
so the flake cannot rot unnoticed. `glib-networking` is in the closure deliberately:
without it everything builds and then the webview cannot complete a TLS handshake,
which reads like a broken network rather than a missing module.

For Fedora, Arch and other distributions, see the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

### Install and run

```powershell
npm ci
npm run desktop
```

`npm run desktop` starts Vite at `127.0.0.1:1420` and builds the Tauri shell with the `desktop` feature. `npm run dev` runs only the frontend in the browser, where development fixtures demonstrate the interface. Use `?empty` for the initial empty state and `?error` for a simulated scan failure. Production browser builds never present fixture data as real scans; local scanning requires the desktop app.

### Build troubleshooting

Common setup failures:

| What you see | What it means |
| --- | --- |
| `cargo metadata` fails with `No such file or directory (os error 2)` | Rust is not installed, or `cargo` is not on the `PATH` of this shell. The path is missing, not the file. Install rustup, then run `source "$HOME/.cargo/env"` or open a new terminal. |
| `failed to run custom build command for 'glib-sys'`, or pkg-config errors naming `webkit2gtk`, `javascriptcoregtk` or `soup` | The Linux system libraries above are missing. Install them and build again. |
| Advice to run `sudo apt install` on a machine that has no `apt` | You are on NixOS outside the development shell. Run `nix develop` first; the check recognises NixOS and should say so itself. |

### Version pinning

The Rust minimum version is declared in `src-tauri/Cargo.toml`. `.cargo/config.toml` enables the compatible-version resolver fallback. The Tauri frontend packages are pinned in `package.json`; use `npm ci` to install the locked dependency versions.

When upgrading Tauri, check the compatibility of the Rust crates, frontend API and plugins together. Update the toolchain requirement and lockfiles as needed, then run the verification checks below.

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

Discovery walks the root breadth-first and bounded: a directory holding a `composer.json` is a project and is never descended into, nothing deeper than four levels is read, `vendor`, `node_modules`, `target` and `dist` are skipped, and a 4000-directory budget ends an oversized walk with a warning instead of a freeze. A root grouped by team or client is therefore the same thing to the rest of the app as a flat one.

## Data contract (IPC)

The frontend communicates with Rust through Tauri commands. Core command signatures (shown as TypeScript notation):

```ts
invoke<ScanResult>('scan_projects', { developmentRoot: string })
invoke<ProjectStatus>('project_status', { projectPath: string })
invoke<ScanResult>('activate_local',  { developmentRoot: string, projectId: string, packageName: string })
invoke<ScanResult>('activate_vendor', { developmentRoot: string, projectId: string, packageName: string })
invoke<ContainerReport>('inspect_container', { developmentRoot: string, projectId: string })
invoke<ContainerReport>('sync_container_mounts', { developmentRoot: string, projectId: string })
invoke<void>('set_php_service', { developmentRoot: string, projectPath: string, service: string | null })
invoke<ScanResult>('apply_profile', { developmentRoot: string, entries: ProfileEntry[] })
invoke<ScanResult>('recover_project', { developmentRoot: string, projectId: string })
invoke<ContainerReport>('apply_container_mounts', { developmentRoot: string, projectId: string, service: string })
invoke<ScanResult>('relink_packages', { developmentRoot: string, projectId: string | null })
invoke<ArchiveEntry[]>('project_history', { developmentRoot: string, projectId: string })
invoke<ArchiveEntry[]>('delete_archive', { developmentRoot: string, projectId: string, packageName: string, archiveId: string })
invoke<ScanResult>('restore_archive', { developmentRoot: string, projectId: string, packageName: string, archiveId: string })
invoke<PreflightReport>('preflight_checks', { developmentRoot: string, projectId: string })
invoke<AutoloadDump>('dump_autoload', { developmentRoot: string, projectId: string })
invoke<ExternalTool[]>('external_tools')
invoke<void>('open_external', { developmentRoot: string, projectId: string, tool: ExternalTool })
invoke<void>('watch_project', { developmentRoot: string, projectId: string })
invoke<void>('stop_watching')
```

`dump_autoload` is the only command that runs Composer, and it runs only
`dump-autoload`: in the project's PHP service when its container answers, on the host
otherwise, under the development-root lock either way. `open_external` starts a program
from a fixed per-platform catalogue and passes the directory as an argument or as the
working directory, never as part of a command string. `watch_project` keeps one watcher
in Tauri state - replacing it stops the previous one - and reports through an event
rather than through a return value:

```ts
listen<{ projectId: string; change: 'branch' | 'vendor' }>('project-changed', handler)
```

A `vendor` change means a scan, a `branch` change means a status refresh, and the
frontend drops its five-minute interval while a watch is established.

`relink_packages` takes `projectId: null` for the whole workspace. The two archive
commands name an archive by its directory name, and the backend only ever acts on an
entry it found by reading that directory itself, so a name from the webview cannot
become a path of its own. `delete_archive` is the single destructive command in the
app and answers with what is left; `preflight_checks` writes a probe wherever a switch
would and removes it again.

A `projectId` is a project's path below the development root with forward slashes - the directory name itself in a flat root, `<group>/<name>` in a grouped one - which is what keeps it unique when two groups hold a project of the same name. `Project.group` carries the directories above it, so the interface can show the grouping without parsing the id. The generated Compose override of a grouped project is nested to match (`<root>/.symfolinker/compose/<group>/<name>/<service>.json`), and every directory along that path is checked for links before it is written.

The two switch commands take **ids, never paths**. They resolve the project and its local source by rescanning the development root, so the webview cannot name an arbitrary directory for a mutation. Both return the scan taken after the switch, which is what the interface renders: there is no optimistic state for a filesystem change (plan section 48).

Scanning and status inspection run on blocking worker threads so the webview stays responsive. Rust serializes domain fields in `camelCase`. TypeScript counterparts are defined in [`src/types/index.ts`](../src/types/index.ts).

Errors, scan warnings and optional Docker messages use this envelope:

```json
{
  "key": "{name}: composer.json is invalid or unreadable; project skipped.",
  "params": { "name": "my-project" }
}
```

The frontend translates the key at render time and inserts the parameters. Raw operating system errors are not exposed. External Docker diagnostic text is included as a parameter in a translated error message.

The webview is restricted by the CSP in `src-tauri/tauri.conf.json`. The capability in `src-tauri/capabilities/default.json` allows `core:default` and `dialog:allow-open`, providing the directory picker.

## Translations

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

## Project structure

| Location | Purpose |
| --- | --- |
| `src/` | Vue interface, Pinia workspace state, translations and TypeScript models. |
| `src-tauri/src/` | Rust discovery, guarded backup/link operations, recovery, profiles and container integration, plus the checks a switch needs: `relink`, `history`, `preflight`, `autoload` and `constraint`. |
| `src-tauri/tests/` | Backend integration tests using temporary workspaces. |
| `tests/` | Translation, scan cache and interface interaction tests. |
| `public/` and `src-tauri/icons/` | Bundled fonts, licenses and application icons. |
| `.github/workflows/` | Cross-platform verification and release builds. |

The Rust crate is split into a library (`symfolinker_lib`) and a binary. Tauri is behind the optional `desktop` feature, so the backend can be tested without webview dependencies. Filesystem locking uses `fs4`; relative links use `pathdiff`, with a junction fallback on Windows.

## Verification

```powershell
npm run build
npm test
npm run test:i18n
npm run test:cache
npm run format:check
npm run check:native
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --no-default-features -- -D warnings
```

Backend tests use temporary workspaces to cover discovery and dependency mapping, path boundaries, locking, backup archiving, local/vendor round trips, rollback, interrupted-operation recovery and profile application. They also cover Git and Compose parsing, engine detection, mount synchronization, translated messages and bounded subprocess execution.

On Windows, junction tests run without elevated permissions. Tests requiring real symlinks are ignored by default because they need Developer Mode or symlink privileges. With those enabled, include them with:

```powershell
npm test -- -- --include-ignored
```

Translation tests check catalog keys, placeholders, interpolation, fallback and language persistence. Cache and interaction tests cover scan restoration, invalid cache entries, selection changes, late responses, failed readback, saved workspaces and profiles.

CI runs frontend builds, translation/cache/interaction tests, Rust tests, formatting and Clippy on Linux, macOS and Windows. It also checks Clippy with the desktop feature on Linux.

## Releasing

Installers are built by [`.github/workflows/release.yml`](../.github/workflows/release.yml), never by hand: each platform has to build on its own runner because an installer embeds that operating system's webview and native libraries.

After updating the version fields, create and push a matching `v<version>` Git tag.

That builds Windows, Linux, and both macOS architectures, then opens a **draft** release with the artifacts attached, so it can be checked before anyone can download it. Publish the draft to make it live.

Bump `version` in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` to match the tag before tagging. The flake reads the version out of `Cargo.toml`, so it is not a fourth place to change.

NixOS installs from the tag rather than from an attached file, because a prebuilt
installer cannot run there: `nix profile install github:JanVDB2000/symfo-linker-profiler/v<version>`.
Pushing the tag is therefore the whole release for NixOS, and no asset corresponds to it.

## Contributing

Issues and pull requests are welcome. Before opening a pull request, run the checks in [Verification](#verification); CI runs the frontend build, translation/cache/interaction tests, Rust tests, formatting and Clippy on Linux, macOS and Windows, and checks the desktop feature on Linux.

Two conventions worth knowing:

- Every mutating filesystem call goes through `WriteGuard` and takes the development-root lock as a parameter, so neither can be forgotten. Do not bypass them.
- The hard rules in [the implementation plan](SymfoLinker-implementation-plan.md) (sections 2.1 to 2.5) are not style preferences; they are what keeps a failed swap from losing someone's work.

## Trademarks

SymfoLinker is an independent project. It is **not affiliated with, endorsed by, or sponsored by Symfony SAS**.

Symfony is a trademark of Symfony SAS. The interface deliberately mirrors the look of the Symfony WebProfilerBundle, whose code is MIT licensed, and the name refers to Symfony only to describe what this tool works with. The [Symfony trademark policy](https://symfony.com/trademark) covers the name and logo separately from that MIT license, so no Symfony logo or brand mark is used anywhere in this application or its icons.

## Technical references

- [Tauri 2: Vite configuration](https://v2.tauri.app/start/frontend/vite/)
- [Tauri 2: platform prerequisites](https://v2.tauri.app/start/prerequisites/)
- [Vue 3: TypeScript guidelines](https://vuejs.org/guide/typescript/overview)
- [Composer: composer.json schema](https://getcomposer.org/doc/04-schema.md)
- [Symfony: Profiler and WebProfilerBundle](https://symfony.com/doc/current/profiler.html)
- [docs/SymfoLinker-implementation-plan.md](SymfoLinker-implementation-plan.md): full specification with hard rules, error types and test cases.
