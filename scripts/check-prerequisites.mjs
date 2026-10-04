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

const RUSTUP_UNIX = `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"`

const APT_PACKAGES = `sudo apt update
sudo apt install build-essential libwebkit2gtk-4.1-dev libxdo-dev \\
  libssl-dev libayatana-appindicator3-dev librsvg2-dev`

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
  const install = process.platform === 'win32'
    ? 'Install Rust from https://rustup.rs, then open a new terminal.'
    : `${RUSTUP_UNIX}\n\nThe second line matters: rustup only adds cargo to the PATH of new\nshells, so without it this terminal still cannot find it.`
  fail('Rust is required to build the desktop app, and cargo was not found.', install)
}

// Tauri renders through the system webview, so Linux needs its development
// headers. pkg-config is how the Rust build locates them; if pkg-config itself
// is missing we cannot tell, so say nothing rather than guess.
if (process.platform === 'linux' && responds('pkg-config', ['--version'])) {
  if (!responds('pkg-config', ['--exists', 'webkit2gtk-4.1'])) {
    fail('The WebKitGTK development files are required and were not found.', APT_PACKAGES)
  }
}
