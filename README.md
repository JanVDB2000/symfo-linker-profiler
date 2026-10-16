# SymfoLinker

[![CI](https://github.com/JanVDB2000/symfo-linker-profiler/actions/workflows/ci.yml/badge.svg)](https://github.com/JanVDB2000/symfo-linker-profiler/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Switch Composer dependencies between the installed vendor version and a local checkout.
SymfoLinker is a desktop app for PHP developers working on a package and its consuming application together, with the original Composer package preserved and project configuration unchanged.

[Download](https://github.com/JanVDB2000/symfo-linker-profiler/releases/latest) | [User guide](docs/usage.md) | [Development](docs/development.md) | [Report an issue](https://github.com/JanVDB2000/symfo-linker-profiler/issues)

<img width="1262" height="916" alt="SymfoLinker desktop interface showing Composer projects and local dependency links" src="https://github.com/user-attachments/assets/6fa258f5-12ea-47b8-b294-0c95f2902dec" />

## Download

Get an installer from the **[latest release](https://github.com/JanVDB2000/symfo-linker-profiler/releases/latest)**.

| Platform | Installer |
| --- | --- |
| Windows | `.msi` or `.exe` |
| macOS | `.dmg` for Apple silicon or Intel |
| Linux | `.AppImage`, `.deb` or `.rpm` |

Builds are unsigned, so your operating system may show a security warning.
See the [installation notes](docs/usage.md#installation) for details.

## Features

- **Local development:** discover Composer projects and switch dependencies with a `VENDOR` / `LOCAL` toggle.
- **Restore vendor packages:** preserve the Composer version and switch back without reinstalling it.
- **Workspace overview:** see Git branches, changed files and active links across projects.
- **Container support:** inspect Docker or Podman services and keep local source mounts in sync.
- **Saved setups:** reopen workspaces and apply named link profiles; choose English, Dutch, French or German.
- **Fits a working day:** regenerate the autoloader where the warning appears, see a `composer install` or a branch change the moment it happens, and open a project in PhpStorm, VS Code, a terminal or the file manager.
- **Catches what breaks links:** restore the links a `composer install` wrote over, undo the last switch, see which preserved versions are kept and what they cost, and check write permissions before a switch rather than during one.

## Quick start

1. Install and open SymfoLinker.
2. Select the directory **above your projects** and scan it.
3. Choose an application and switch a dependency to **LOCAL**, then confirm.
4. Switch back to **VENDOR** when you want the preserved Composer version.

For example, select `dev/` with this layout:

```text
dev/
  app/
    composer.json       # requires "acme/bundle"
    vendor/
  bundle/
    composer.json       # name: "acme/bundle"
```

A root grouped by team, client or kind works just as well, because a project is
wherever its `composer.json` is:

```text
dev/
  apps/
    app/
      composer.json     # requires "acme/bundle"
      vendor/
  library/
    bundle/
      composer.json     # name: "acme/bundle"
  docker-nginx-proxy/   # no manifest, so not a project
```

The scan looks up to four directories below the selected root, stops at every
directory that holds a `composer.json`, and never enters `vendor/` or `node_modules/`.
Projects are listed under the group they sit in, and each one is identified by its path
below the root, so two groups may hold a project of the same name.
Packages are matched by their Composer names in `require` and `require-dev`.

### Backups and containers

The original package is kept in `vendor/.symfolinker/` while its local checkout is linked, and an earlier version it replaces moves to `vendor/.symfolinker/.history/` instead of being overwritten. Nothing is ever deleted automatically; **Health > Preserved versions** is where a kept version is restored or removed by hand.
Composer and project Compose files stay unchanged; app state and generated Compose overrides live in `<root>/.symfolinker/`.

When local source mounts change, SymfoLinker may recreate the running PHP service.
Mounted volumes are reused, but data stored only in the container's writable layer is not preserved.
Read about [backups, profiles and containers](docs/usage.md#backups-profiles-and-containers) before using these features.

## Development

Requires Node.js **22.12+**, npm, Rust **1.87+**, and the platform's Tauri build prerequisites.
Git provides branch and working-directory information.

From a checkout of this repository:

```bash
npm ci
npm run desktop
```

To build installers:

```bash
npm run desktop:build
```

`npm run dev` opens a browser demo; scanning local projects requires the desktop app.
See the [development guide](docs/development.md#getting-started) for platform setup, troubleshooting and verification commands.

## Documentation & contributing

- [User guide](docs/usage.md): installation, workspaces, switching, recovery and container behavior.
- [Development guide](docs/development.md): setup, architecture, safety safeguards, tests and releases.
- [Roadmap](docs/roadmap.md): what is done, what is next and what is deliberately out of scope.
- [Implementation plan](docs/SymfoLinker-implementation-plan.md): background specification and design rules.

Issues and pull requests are welcome. Run the [verification checks](docs/development.md#verification) before submitting a pull request.

## License

[MIT](LICENSE). Bundled assets retain their own licenses:
[JetBrains Mono](public/fonts/LICENSE.txt) and [Symfony profiler styling](public/SYMFONY-LICENSE.txt).

SymfoLinker is an independent project, not affiliated with, endorsed by or sponsored by Symfony SAS.
See [trademark details](docs/development.md#trademarks).
