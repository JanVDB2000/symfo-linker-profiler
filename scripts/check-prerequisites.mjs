// Fails early with a readable message when the desktop toolchain is missing.
//
// Without this, a machine without Rust gets the Tauri CLI's own diagnosis:
//
//   failed to run `cargo metadata` command to get workspace directory:
//   No such file or directory (os error 2)
//
// which names neither Rust nor the fix, and reads as a missing file rather than
// a missing program. Node built-ins only, so it works before anything is set up.
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'

const RUSTUP_UNIX = `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"`

// Only what the build actually links against: neither libxdo, nor OpenSSL, nor an
// appindicator appears in Cargo.lock, and this app registers no tray and no global
// shortcut, so naming those packages sent people installing things they do not need.
const APT_PACKAGES = `sudo apt update
sudo apt install build-essential libwebkit2gtk-4.1-dev librsvg2-dev`

// On NixOS every answer above is the wrong one: there is no apt, and the rustup
// tarball's dynamically linked binaries do not run against a non-FHS loader. The
// flake in this repository carries the toolchain and the system libraries together,
// so the fix there is to enter it rather than to install anything.
const NIX_SHELL = `nix develop

Everything the desktop build needs - Rust, Node, WebKitGTK and GTK - comes from
the flake in this repository, so nothing has to be installed first.`

/** True on NixOS, where the usual package-manager advice does not apply. */
function isNixOs() {
  // /etc/NIXOS is the marker NixOS itself writes. NIX_STORE also catches a Nix shell
  // on another distribution, where the same advice happens to be the right one.
  return existsSync('/etc/NIXOS') || !!process.env.NIX_STORE
}

/** True when the command exists and exits cleanly. */
function responds(command, args) {
  try {
    execFileSync(command, args, { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

function fail(title, body) {
  console.error(`\n  ${title}\n`)
  for (const line of body.split('\n')) console.error(`    ${line}`)
  console.error('')
  console.error('  Full setup instructions: see "Requirements" in README.md.\n')
  process.exit(1)
}

if (!responds('cargo', ['--version'])) {
  let install
  if (process.platform === 'win32') {
    install = 'Install Rust from https://rustup.rs, then open a new terminal.'
  } else if (isNixOs()) {
    install = NIX_SHELL
  } else {
    install = `${RUSTUP_UNIX}\n\nThe second line matters: rustup only adds cargo to the PATH of new\nshells, so without it this terminal still cannot find it.`
  }
  fail('Rust is required to build the desktop app, and cargo was not found.', install)
}

// Tauri renders through the system webview, so Linux needs its development
// headers. pkg-config is how the Rust build locates them; if pkg-config itself
// is missing we cannot tell, so say nothing rather than guess.
if (process.platform === 'linux' && responds('pkg-config', ['--version'])) {
  if (!responds('pkg-config', ['--exists', 'webkit2gtk-4.1'])) {
    fail(
      'The WebKitGTK development files are required and were not found.',
      isNixOs() ? NIX_SHELL : APT_PACKAGES,
    )
  }
}
