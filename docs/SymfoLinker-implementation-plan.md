# SymfoLinker — Implementation Plan

> A local GUI app for safe Symfony/Composer bundle development with symlinks, vendor backups, Git status and Docker/PHP-FPM awareness.

---

## 1. Goal

SymfoLinker is a local desktop app that manages multiple PHP/Composer projects under a single development root.

Example:

```text
~/dev/
├── .symfolinker/
├── project-a/
├── project-b/
├── project-c/
└── project-d/
```

For example, `project-a` uses:

```text
company/project-b
company/project-c
company/project-d
```

through Composer.

During development, you want to switch between:

```text
VENDOR
project-a/vendor/company/project-b
```

and:

```text
LOCAL
project-a/vendor/company/project-b
    -> ~/dev/project-b
```

without losing the original Composer version.

---

# 2. Hard rules

These rules are fundamental and must be enforced in code.

## 2.1 Projects are read-only outside `vendor/`

SymfoLinker may read project files, but must not modify them.

Never modify these automatically:

```text
composer.json
composer.lock
.gitignore
.env
.env.local
compose.yaml
compose.yml
docker-compose.yaml
docker-compose.yml
config/
src/
tests/
.git/
```

The only mutation zone inside a project is:

```text
<project>/vendor/
```

---

## 2.2 App state lives above the projects

SymfoLinker uses:

```text
~/dev/.symfolinker/
```

for its own state.

For example:

```text
~/dev/.symfolinker/
├── config.json
├── state.json
├── lock
└── logs/
```

No `.symfolinker` directory is created at the root of an individual project.

---

## 2.3 Always preserve the original vendor package

Activating a local package must not delete the original Composer directory.

For:

```text
project-a/vendor/company/project-b
```

first becomes:

```text
project-a/vendor/company/project-b
        ↓ move
project-a/vendor/.symfolinker/company/project-b
```

Then:

```text
project-a/vendor/company/project-b
    -> ../../../project-b
```

The backup must be preserved for as long as the local version is active.

---

## 2.4 No destructive fallback

SymfoLinker must never automatically use:

```bash
rm -rf vendor/company/project-b
composer install
composer update
git reset --hard
git clean
git stash
```

to "fix" an error.

If a safe swap is not possible:

```text
STOP
```

and show the error to the user.

---

## 2.5 Atomicity

Every mutation must support rollback.

If:

```text
create backup → succeeds
create symlink → fails
```

then:

```text
backup → restore original vendor location
```

must be performed.

The user must never be left with a partially completed swap.

---

# 3. Recommended stack

## Desktop shell

```text
Tauri
```

## Frontend

```text
Vue 3
TypeScript
Vite
Pinia
```

## Native backend

```text
Rust
```

## External tools that may be invoked

```text
git
docker
docker compose
```

In the MVP, Composer is inspected only through files:

```text
composer.json
composer.lock
vendor/composer/installed.json
```

---

# 4. Why Tauri

SymfoLinker primarily performs local system operations:

```text
filesystem
symlinks
atomic rename
Git inspection
Docker inspection
process execution
path mapping
file locking
```

Tauri is suitable because:

- the app can remain small;
- filesystem work runs natively;
- a full Electron runtime is unnecessary;
- Rust is suitable for safe filesystem operations;
- the entire GUI can be built in Vue.

---

# 5. Repository structure

Start with:

```text
symfolinker/
├── src/
│   ├── App.vue
│   ├── main.ts
│   ├── router/
│   ├── stores/
│   ├── components/
│   ├── views/
│   └── types/
│
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/
│   │   │   ├── scan.rs
│   │   │   ├── project.rs
│   │   │   ├── package.rs
│   │   │   ├── linker.rs
│   │   │   ├── git.rs
│   │   │   └── docker.rs
│   │   │
│   │   ├── domain/
│   │   │   ├── project.rs
│   │   │   ├── package.rs
│   │   │   ├── link.rs
│   │   │   ├── backup.rs
│   │   │   ├── git.rs
│   │   │   └── runtime.rs
│   │   │
│   │   ├── services/
│   │   │   ├── discovery.rs
│   │   │   ├── composer.rs
│   │   │   ├── linker.rs
│   │   │   ├── backup.rs
│   │   │   ├── git.rs
│   │   │   ├── docker.rs
│   │   │   ├── path_mapper.rs
│   │   │   ├── state.rs
│   │   │   └── write_guard.rs
│   │   │
│   │   └── errors/
│   │       └── mod.rs
│   │
│   └── Cargo.toml
│
├── package.json
├── vite.config.ts
└── README.md
```

---

# 6. Development root

On first launch, the user chooses one development root.

For example:

```text
/home/user/dev
```

SymfoLinker creates only:

```text
/home/user/dev/.symfolinker/
```

for its own files.

Config:

```json
{
  "developmentRoot": "/home/user/dev"
}
```

---

# 7. Project discovery

In the first version, scan only immediate child directories:

```text
~/dev/*
```

A directory is a Composer project when:

```text
composer.json
```

exists.

Example:

```text
~/dev/project-a/composer.json
~/dev/project-b/composer.json
```

Read at least:

```json
{
  "name": "company/project-b",
  "require": {},
  "require-dev": {}
}
```

---

# 8. Project model

Rust model:

```rust
pub struct Project {
    pub id: String,
    pub name: String,
    pub composer_name: Option<String>,
    pub path: PathBuf,
    pub dependencies: Vec<Dependency>,
    pub git: Option<GitInfo>,
    pub docker: Option<DockerInfo>,
}
```

Example:

```json
{
  "id": "project-b",
  "name": "project-b",
  "composerName": "company/project-b",
  "path": "/home/user/dev/project-b"
}
```

---

# 9. Dependency model

```rust
pub struct Dependency {
    pub package_name: String,
    pub constraint: String,
    pub dependency_type: DependencyType,
    pub local_project_id: Option<String>,
}
```

Types:

```rust
pub enum DependencyType {
    Require,
    RequireDev,
}
```

By default, the GUI only needs to display dependencies that have a matching local Composer project.

---

# 10. Package mapping

Build an index during the scan:

```text
company/project-a -> /home/user/dev/project-a
company/project-b -> /home/user/dev/project-b
company/project-c -> /home/user/dev/project-c
```

This allows:

```text
project-a require company/project-b
```

to be mapped automatically to:

```text
/home/user/dev/project-b
```

---

# 11. Package status

For each locally available dependency, determine:

```text
vendor path
local path
backup path
link status
backup status
git status
docker status
```

Model:

```rust
pub struct PackageStatus {
    pub package_name: String,
    pub vendor_path: PathBuf,
    pub local_path: PathBuf,
    pub backup_path: PathBuf,
    pub mode: PackageMode,
    pub link_status: LinkStatus,
    pub backup_status: BackupStatus,
    pub git: Option<GitInfo>,
    pub runtime: RuntimeStatus,
}
```

---

# 12. Package modes

```rust
pub enum PackageMode {
    Vendor,
    Local,
    Unknown,
}
```

## Vendor

```text
vendor/company/project-b
```

is a real directory.

## Local

```text
vendor/company/project-b
```

is a symlink.

## Unknown

The state is inconsistent and must not be changed automatically.

---

# 13. Link status

```rust
pub enum LinkStatus {
    NotLinked,
    Linked,
    Broken,
    UnexpectedTarget,
    ContainerInvalid,
}
```

---

# 14. Backup status

```rust
pub enum BackupStatus {
    Missing,
    Available,
    Invalid,
    Stale,
}
```

---

# 15. Backup location

For package:

```text
company/project-b
```

in:

```text
project-a
```

the backup is:

```text
project-a/vendor/.symfolinker/company/project-b
```

Therefore:

```text
vendor/
├── .symfolinker/
│   └── company/
│       └── project-b/
│
└── company/
    └── project-b -> ../../../project-b
```

---

# 16. Link operation

Command:

```text
activate_local(project_id, package_name)
```

Flow:

```text
1. retrieve project
2. check package dependency
3. check local project
4. calculate vendor path
5. calculate backup path
6. validate WriteGuard
7. inspect current status
8. perform Docker/path pre-validation
9. check backup
10. atomically move vendor directory to backup
11. create relative symlink
12. host validation
13. container validation
14. update state
```

---

# 17. Link preconditions

`activate_local` may proceed only when:

```text
local project exists
vendor package exists
vendor path is not an unexpected symlink
backup path is safe
vendor and backup are inside vendor/
container validation is possible or Docker is inactive
```

---

# 18. Creating a vendor backup

Pseudocode:

```rust
if backup_exists {
    return error_if_not_known_safe();
}

rename(vendor_path, backup_path)?;
create_symlink(relative_target, vendor_path)?;
```

Use `rename` instead of copy + delete wherever possible.

This is faster and safer when the source and destination are on the same filesystem.

---

# 19. Rollback

Pseudocode:

```rust
let moved = move_vendor_to_backup();

match create_link() {
    Ok(_) => {}
    Err(error) => {
        restore_backup_to_vendor();
        return Err(error);
    }
}
```

Container validation after creating the symlink must also be able to trigger rollback.

---

# 20. Restore vendor

Command:

```text
activate_vendor(project_id, package_name)
```

Flow:

```text
1. verify the vendor path is a managed symlink
2. check backup
3. remove symlink
4. atomically restore backup
5. validate result
6. update state
```

---

# 21. Swap

The frontend can use a single toggle:

```text
VENDOR  ⇄  LOCAL
```

The backend explicitly uses:

```text
activate_local(...)
activate_vendor(...)
```

Do not create a backend command that blindly performs a "toggle".

The GUI first determines the current state, then explicitly requests the desired target state.

This makes error handling clearer.

---

# 22. WriteGuard

Make this a central safety component.

```rust
pub struct WriteGuard {
    development_root: PathBuf,
}
```

Every mutating filesystem operation must first pass:

```rust
write_guard.assert_vendor_path(project_path, target_path)?;
```

Allowed:

```text
/project-a/vendor/company/project-b
/project-a/vendor/.symfolinker/company/project-b
```

Not allowed:

```text
/project-a/composer.json
/project-a/src/Service.php
/project-a/.git/config
```

On violation:

```text
WriteDenied
```

---

# 23. Symlink strategy

Use relative links by default.

For:

```text
/home/user/dev/project-a/vendor/company/project-b
```

to:

```text
/home/user/dev/project-b
```

calculate the relative path.

For example:

```text
../../../project-b
```

Never hardcode it.

Use a path-diff library or implement safe relative path calculation.

---

# 24. Why relative links

Host:

```text
/home/user/dev
```

Container:

```text
/var/www
```

An absolute link to:

```text
/home/user/dev/project-b
```

will probably not work inside the container.

A relative link can work in both environments when the same directory structure is mounted.

---

# 25. Git integration

Git is read-only in the MVP.

Use:

```bash
git -C <path> branch --show-current
git -C <path> rev-parse --short HEAD
git -C <path> status --porcelain
```

Model:

```rust
pub struct GitInfo {
    pub branch: String,
    pub commit: String,
    pub dirty: bool,
    pub changed_files: usize,
}
```

---

# 26. Docker discovery

Look for these files without modifying them:

```text
compose.yaml
compose.yml
docker-compose.yaml
docker-compose.yml
```

inside the active project.

Optionally run:

```bash
docker compose config
```

to determine the effective volumes and services.

SymfoLinker never modifies Docker configuration.

---

# 27. PHP-FPM service

First version:

- the user can select the PHP service;
- suggest a service automatically based on its name.

Possible names:

```text
php
php-fpm
app
backend
web
```

Save the selection outside the project:

```text
~/dev/.symfolinker/config.json
```

For example:

```json
{
  "projects": {
    "/home/user/dev/project-a": {
      "phpService": "php-fpm"
    }
  }
}
```

---

# 28. Docker path mapping

For a volume:

```text
/home/user/dev -> /var/www
```

create:

```rust
pub struct VolumeMapping {
    pub host_path: PathBuf,
    pub container_path: PathBuf,
}
```

API:

```rust
fn to_container_path(host_path: &Path) -> Option<PathBuf>;
```

Example:

```text
/home/user/dev/project-b
→
/var/www/project-b
```

---

# 29. Container validation

After linking, check the exact vendor link inside the container.

For example:

```bash
docker compose exec -T php-fpm test -e /var/www/project-a/vendor/company/project-b
```

Then:

```bash
docker compose exec -T php-fpm readlink /var/www/project-a/vendor/company/project-b
```

The first version does not need to support complex shell expressions.

Pass subprocess arguments directly in Rust.

---

# 30. No Docker

If no Docker Compose configuration is found:

```text
Runtime: Native
```

Only host validation is then required.

---

# 31. State

Use state only for metadata and ownership.

For example:

```json
{
  "version": 1,
  "links": {
    "/home/user/dev/project-a|company/project-b": {
      "projectPath": "/home/user/dev/project-a",
      "package": "company/project-b",
      "localPath": "/home/user/dev/project-b",
      "backupPath": "/home/user/dev/project-a/vendor/.symfolinker/company/project-b",
      "managed": true
    }
  }
}
```

Important:

The actual filesystem state remains authoritative.

Stored state must never be the only source used to restore a package.

---

# 32. File locking

Use:

```text
~/dev/.symfolinker/lock
```

for mutating actions.

Goal:

```text
prevent two swaps from modifying the same vendor simultaneously
```

Initially, a single global lock per development root is sufficient.

---

# 33. Tauri commands

Start with these commands.

## Scan

```text
scan_projects()
```

Return:

```text
Project[]
```

## Project details

```text
get_project(project_id)
```

## Package status

```text
get_package_status(project_id, package_name)
```

## All package statuses

```text
get_project_packages(project_id)
```

## Activate local

```text
activate_local(project_id, package_name)
```

## Activate vendor

```text
activate_vendor(project_id, package_name)
```

## Git refresh

```text
get_git_status(project_id)
```

## Docker inspection

```text
inspect_runtime(project_id)
```

## Health

```text
run_health_check(project_id)
```

---

# 34. Frontend pages

Use at least:

```text
Projects
Links
Runtime
Health
Settings
```

---

# 35. Projects view

Layout:

```text
┌───────────────────┬─────────────────────────────────────────────┐
│ PROJECTS          │ project-a                                   │
│                   │                                             │
│ ● project-a       │ Runtime: Docker / php-fpm                   │
│   project-b       │ Branch: feature/foo                         │
│   project-c       │                                             │
│                   │ Packages                                    │
│                   │                                             │
│                   │ company/project-b   LOCAL   feature/foo ✓   │
│                   │ company/project-c   VENDOR  main        ✓   │
│                   │ company/project-d   LOCAL   develop     ⚠   │
└───────────────────┴─────────────────────────────────────────────┘
```

---

# 36. Package card

For each package:

```text
company/project-b

Mode
[ Vendor ] [ Local ]

Local project
/home/user/dev/project-b

Git
feature/foo
3 modified files

Vendor backup
✓ available

Host
✓ valid

PHP-FPM
✓ valid
```

---

# 37. Confirmation dialogs

When switching to local:

```text
Use local package?

company/project-b

The original Composer package will be preserved in:

vendor/.symfolinker/company/project-b

Local source:

/home/user/dev/project-b

[ Cancel ] [ Use Local ]
```

When restoring:

```text
Restore vendor package?

company/project-b

The local symlink will be removed and the preserved
Composer package restored.

[ Cancel ] [ Restore Vendor ]
```

---

# 38. Links view

Show all active links across all projects.

```text
ACTIVE LINKS

project-a
  company/project-b
  → /home/user/dev/project-b
  branch: feature/foo

admin-app
  company/project-b
  → /home/user/dev/project-b
  branch: feature/foo
```

This shows immediately where a local bundle is actively used.

---

# 39. Runtime view

For the selected project:

```text
Docker Compose
● running

PHP service
php-fpm

Host project
/home/user/dev/project-a

Container project
/var/www/project-a

Mounts
/home/user/dev → /var/www
```

Package validation:

```text
company/project-b
Host      ✓
PHP-FPM   ✓
```

---

# 40. Health view

Example:

```text
Environment

✓ development root
✓ project readable
✓ vendor writable
✓ Git available
✓ Docker available
✓ php-fpm running

Packages

✓ company/project-b
⚠ company/project-c backup stale
✗ company/project-d unavailable in container
```

---

# 41. Error types

Define explicit error types.

```rust
pub enum SymfoLinkerError {
    ProjectNotFound,
    ComposerJsonInvalid,
    PackageNotFound,
    LocalProjectNotFound,
    VendorPackageMissing,
    BackupAlreadyExists,
    BackupMissing,
    BackupInvalid,
    UnexpectedSymlink,
    BrokenSymlink,
    WriteDenied,
    DockerUnavailable,
    DockerServiceUnavailable,
    ContainerPathUnavailable,
    GitUnavailable,
    LockUnavailable,
    IoError,
}
```

The frontend displays clear user-facing messages.

---

# 42. No raw backend errors in the GUI

Instead of:

```text
Os { code: 13, kind: PermissionDenied ... }
```

Show:

```text
SymfoLinker cannot modify this vendor package.

Path:
/home/user/dev/project-a/vendor/company/project-b

Reason:
Permission denied.
```

---

# 43. Logging

Logs live above the projects:

```text
~/dev/.symfolinker/logs/
```

Log:

```text
timestamp
action
project
package
source path
destination path
result
rollback
```

Never log source code or secrets.

---

# 44. Security

Treat all paths from projects as untrusted input.

Check:

```text
canonical paths
path traversal
symlink escape
vendor boundary
development-root boundary
```

Example:

A malicious symlink under `vendor/` must not cause SymfoLinker to move or delete a directory outside the allowed area.

Use canonical path checks before mutations wherever possible.

---

# 45. First milestone — read-only scanner

First, build only:

```text
select development root
scan projects
read composer.json
create package mapping
display Git branch
display vendor status
```

No mutations.

Acceptance criteria:

- the app finds all immediate Composer projects;
- package names are mapped correctly;
- dependencies with local projects are displayed;
- Git branch/status is displayed;
- no project file is modified.

---

# 46. Second milestone — safe backup engine

Next, build only the backend for:

```text
vendor → backup
backup → vendor
```

Without symlinks yet.

Write tests for:

```text
happy path
backup already exists
vendor is missing
permission denied
rollback
write guard
```

---

# 47. Third milestone — local symlink

Add:

```text
create relative symlink
inspect symlink
detect broken link
detect unexpected target
```

Acceptance criteria:

```text
vendor → local
local → vendor
```

works without losing the original Composer package.

---

# 48. Fourth milestone — GUI switch

Build the:

```text
VENDOR ⇄ LOCAL
```

switch.

Important:

Update the UI only after the backend confirms success.

Do not use optimistic state for filesystem mutations.

---

# 49. Fifth milestone — Docker

Then add:

```text
compose discovery
service selection
mount inspection
host/container path mapping
container link validation
```

Docker configuration remains read-only.

---

# 50. Sixth milestone — links dashboard

Scan all projects and show:

```text
which local package
in which project
is actively linked
on which branch
```

---

# 51. Tests

At least three types of tests.

## Unit tests

For:

```text
relative path calculation
package mapping
status detection
write guard
path mapper
```

## Filesystem integration tests

Create a temporary structure:

```text
tmp/
├── project-a/
│   └── vendor/
└── project-b/
```

Test real:

```text
rename
symlink
restore
rollback
```

## Docker integration tests

Optional in CI later.

At a minimum, test locally against a simple Compose fixture.

---

# 52. Critical test cases

## Normal link

```text
vendor exists
backup does not exist
local exists
```

Result:

```text
backup present
symlink present
```

## Restore

```text
symlink present
backup present
```

Result:

```text
real vendor directory restored
symlink removed
```

## Backup exists unexpectedly

Result:

```text
no changes
error
```

## Symlink points to the wrong target

Result:

```text
no changes
UnexpectedSymlink
```

## Local project is missing

Result:

```text
no changes
```

## Docker target is missing

Result:

```text
no swap
```

or, when the error is only discovered after linking:

```text
rollback to vendor
```

## Writing to composer.json

Result:

```text
WriteDenied
```

---

# 53. MVP scope

The first usable release includes only:

```text
GUI
development root
project discovery
composer package mapping
Git branch/status
vendor/local status
safe vendor backup
relative symlinks
vendor restore
active links dashboard
Docker Compose detection
PHP service selection
container validation
health screen
```

Outside the MVP:

```text
execute Composer commands
branch switching
Git writes
DDEV
Lando
Podman
automatically modify Docker configuration
modify composer.json
automatically add a dependency
```

---

# 54. Initial implementation order

Work in this order:

```text
1. start Tauri + Vue project
2. development root selector
3. Rust ProjectDiscovery
4. composer.json parser
5. Project + Dependency models
6. package index
7. Projects GUI
8. read-only Git service
9. package status detector
10. WriteGuard
11. BackupManager
12. LinkManager
13. rollback tests
14. VENDOR/LOCAL switch
15. Links dashboard
16. Docker discovery
17. PathMapper
18. container validation
19. Health view
20. packaging/release
```

---

# 55. First technical goal

SymfoLinker first becomes useful when this works:

```text
~/dev/
├── project-a
└── project-b
```

where:

```text
project-a requires company/project-b
project-b composer name = company/project-b
```

The GUI shows:

```text
company/project-b

Current: VENDOR
Branch: feature/foo

[ Use Local ]
```

Click:

```text
vendor/company/project-b
→ backup

vendor/company/project-b
→ symlink to project-b
```

GUI:

```text
Current: LOCAL
Vendor backup: ✓
Host: ✓
```

Click:

```text
[ Restore Vendor ]
```

and the original Composer directory is restored exactly.

When this works reliably, the core of SymfoLinker is complete.

---

# 56. Definition of Done for v0.1

SymfoLinker v0.1 is complete when:

- projects are discovered automatically;
- Composer dependencies are mapped to local repositories;
- branch and dirty status are visible;
- local/vendor mode is detected correctly;
- the original vendor directory is always safely preserved;
- local symlinks use relative paths;
- restoration works without Composer;
- failed swaps roll back automatically;
- no writes outside `vendor/` are possible within projects;
- app state lives only above the projects;
- Docker/PHP-FPM link validation works for normal bind mounts;
- the user can switch between local and vendor through the GUI;
- active links across all projects are visible.

---

# 57. Product principle

SymfoLinker must behave as though every project is immutable outside `vendor/`.

The core is:

```text
READ
composer.json
composer.lock
Git
Docker config

WRITE
only vendor/
only safe package swap operations

STORE OWN STATE
only above the projects
```

---

# 58. Short product description

> **SymfoLinker is a local desktop app for safe Composer package development. It discovers local packages, shows Git branches and Docker/PHP-FPM availability, and lets you safely switch between the original Composer vendor version and a local symlink — without changing project configuration.**

---

# 59. Tagline

> **SymfoLinker — safely switch Composer packages between vendor and local development.**

---

# 60. Interface languages

English is the default language for the app and project documentation. The UI offers English (EN), Dutch (NL), French (FR) and German (DE) under **Settings → Appearance → Language**.

Use separate translation catalogs in `../src/i18n/locales`. English source text is the key. All app-owned labels, accessible names, tooltips, empty states, warnings and errors must use these catalogs. Keep internal tab identifiers, package names, paths, Git branches and external command output independent of translated text.

Rust returns `{ key, params }` message envelopes. Translate them in the frontend at render time so an existing warning or error follows a language change without rescanning. Interpolate parameters as text in one pass, use English fallback for missing translations and keep placeholders identical in all catalogs.

Save the selected locale in `localStorage` as `symfolinker.language`. Restore it on launch, default unsupported values to English, update the document's `lang` attribute and format timestamps in the selected locale. Changes apply immediately without restarting the app.

Validate catalog completeness, placeholders, fallback behavior, safe interpolation, language persistence and rendered translated UI with `npm run test:i18n`.
