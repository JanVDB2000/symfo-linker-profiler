# Roadmap

[Back to the README](../README.md) | [User guide](usage.md) | [Development guide](development.md)

What is done, what is next, and why. Items are grouped by the kind of problem they
solve rather than by release: a gap that interrupts a working day outranks a feature
that merely sounds good.

## Contents

- [Shipped](#shipped)
- [Next: gaps that interrupt a working day](#next-gaps-that-interrupt-a-working-day)
- [Daily comfort](#daily-comfort)
- [Larger directions](#larger-directions)
- [Small things](#small-things)
- [Deliberately not planned](#deliberately-not-planned)

## Shipped

The six implementation-plan milestones - read-only scanner, safe backup engine, local
symlink, GUI switch, Docker and Podman, links dashboard - are complete, including
recovery metadata, bounded external commands, saved workspaces and link profiles.

The gaps that used to cost the most time are closed as well:

| Feature | What it answers |
| --- | --- |
| **Replaced links** | A `composer install` that wrote over a link is detected from the journal and restored for the whole workspace in one pass ([`relink.rs`](../src-tauri/src/relink.rs)). |
| **Autoload comparison** | The checkout's autoload rules are compared with the ones the generated autoloader was built from, so "class not found" is predicted rather than debugged ([`autoload.rs`](../src-tauri/src/autoload.rs)). |
| **Version comparison** | The version line a checkout provides is compared with the constraint the project asks for, and says nothing unless it can prove a conflict ([`constraint.rs`](../src-tauri/src/constraint.rs)). |
| **Preserved versions** | Everything a switch set aside is listed with its size, can be restored, and can be removed by hand ([`history.rs`](../src-tauri/src/history.rs)). |
| **Write checks** | Write permission, link kind and free space are probed before a switch instead of failing during one ([`preflight.rs`](../src-tauri/src/preflight.rs)). |
| **Undo last switch** | What the last switch moved is put back through the profile path, with the same validation and rollback. |
| **Regenerate the autoloader** | The autoload comparison reported a mismatch and now repairs it: `composer dump-autoload` runs in the project's PHP service when its container is up, and on the host otherwise, with Composer's own output kept verbatim ([`dump_autoload.rs`](../src-tauri/src/dump_autoload.rs)). The one Composer command this app runs; `install` and `update` stay out, as hard rule 2.4 requires. |
| **A watcher instead of polling** | A `composer install` under `vendor` and a branch moving under `.git` arrive as events, debounced into one refresh, and the five-minute poll steps aside while the watch holds ([`watch.rs`](../src-tauri/src/watch.rs)). Non-recursive on three directories, so a dependency tree is never traversed. |
| **More than one checkout** | The same repository checked out twice under one root - a multi-tenant test beside a QA branch - used to drop the package out of the interface entirely. The candidates are now listed with their branch and commit, and the chosen one is stored per development root ([`discovery.rs`](../src-tauri/src/discovery.rs)). The scanner still never guesses. |
| **A lock that tells the truth** | A filesystem that cannot lock at all, and one copy of the app overlapping itself, both used to report "another SymfoLinker instance is working in this development root". The first is now reported as what it is and work continues; the second queues; and a lock left behind by an interrupted run can be taken over by hand ([`lock.rs`](../src-tauri/src/lock.rs)). |
| **Installable on NixOS** | A prebuilt `.deb` or AppImage cannot run on NixOS, so there is a flake: `nix profile install github:JanVDB2000/symfo-linker-profiler/v0.5.0`, plus `nix develop` for working on it. CI builds the package on every push. |
| **An update check** | Settings reports the running version and whether a newer one has been published, with the instruction that fits how this copy was installed - including that a Nix store path cannot replace itself. One request, on a button, never on its own. |
| **Open in an editor, a terminal or the file manager** | A project directory can be handed to PhpStorm, VS Code, a terminal or the file manager, and its path copied, with only the tools this computer actually has offered ([`external.rs`](../src-tauri/src/external.rs)). Ids in, directories resolved by a scan, and no path ever accepted from the interface. |

## Next: gaps that interrupt a working day

The three that stood here are shipped and have moved into the table above:
regenerating the autoloader, a filesystem watcher instead of polling, and handing a
project to an editor, a terminal or the file manager. What remains in this category is
smaller, and it is listed under Daily comfort below.

## Daily comfort

| Item | Why |
| --- | --- |
| **Tray menu, global shortcut, command palette** | Switching a package without finding the window first. The filter already has `Ctrl+K`; a palette that switches by typing a package name is the same idea one step further. |
| **Desktop notification when a pass finishes** | A mount synchronisation that recreates a container takes long enough to look away from. The in-window banner is invisible when the window is. |
| **Profiles that leave this machine** | Export and import a profile as JSON so a team shares one setup, an indicator when the current state has drifted from a profile, and rename and duplicate. Profiles live only in `localStorage` today. |
| **More than one root at a time** | Saved workspaces exist, but opening another root discards the cache of the previous one. Tabs, or a cache per root, would make switching between clients cheap. |
| **Configurable scan scope** | Grouped roots are scanned as they are now, four levels deep, stopping at every manifest. What is still fixed is the shape of that rule: a deeper tree, exclude globs, and sub-packages inside a project (`packages/*` beneath a monorepo's own `composer.json`) would need Settings to offer it as a choice. |
| **Container actions** | `cache:clear`, the PHP version and loaded extensions, Xdebug status, service logs, restart the service. The connection is already there and verified. |
| **More Git state, still read-only** | Ahead and behind the upstream, the last commit's author and date, the list of changed files. Never `checkout`, `stash` or `reset`: hard rule 2.4. |

## Larger directions

### A command line companion

The crate is already split into a library and a Tauri binary behind the `desktop`
feature, so a `symfolinker status --json`, `link`, `unlink` and `profile apply` would
reuse the same guarded operations. It opens scripts, CI and work over SSH, where a
desktop window is not available.

### The same safety model for npm, pnpm and yarn

`node_modules/@scope/package` has the identical problem, the identical dangerous
workaround, and would reuse the backup engine, the journal and the write guard almost
unchanged. It is a second audience rather than a second feature.

### Updates and diagnostics

The check against the GitHub releases API is shipped and has moved into the table
above. What is still open is applying an update in place, which needs a signing keypair
and a configured `tauri-plugin-updater`: the release workflow already builds macOS
update bundles that nothing consumes. Worth saying that this would never cover every
install - a Nix store path is read only, and a `.deb` belongs to its package manager -
so the per-platform instruction stays either way.

Alongside it, a one-click diagnostics copy - engine versions, Compose state, journal
states, path status - so an issue report starts with facts.

## Small things

- Pin and tag projects, so the sidebar opens on the two that matter today.
- A dry run that lists the planned renames before a first-time switch confirms.
- A session history of switches, beyond the single step undo already present.
- More languages; the four catalogs are already enforced to stay in step.

## Deliberately not planned

- **Editing `composer.json`.** The entire point is to switch without touching project
  configuration, so a `repositories: path` entry is out of scope.
- **Automatic repair.** Recovery, relinking and archive removal are explicit actions.
  Nothing in this app may delete or reinstall a package to resolve an error by itself
  (hard rules 2.3 and 2.4).
- **Signed installers.** A certificate costs money per year; the releases stay
  unsigned, and the [installation notes](usage.md#these-builds-are-not-signed) say so
  plainly.
