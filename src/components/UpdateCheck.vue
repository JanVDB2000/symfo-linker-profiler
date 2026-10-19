<script setup lang="ts">
// Which version is running, and whether a newer one has been published.
//
// The check is explicit, never automatic: this app reaches the network for nothing
// else, and starting to do so unasked would break that. One GET to the public
// releases API, nothing sent, nothing stored.
//
// An update is not one action, because it depends on how SymfoLinker was installed:
// a Windows installer and a macOS bundle replace themselves, an AppImage replaces its
// own file, a distribution package belongs to its distribution, and a Nix store path
// is read only so nothing can replace it at all. The backend says which case this is
// and the panel gives that case's instruction, rather than offering one button that
// would be wrong more often than right.
import { ref } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { getVersion } from '@tauri-apps/api/app'
import Spinner from './Spinner.vue'
import { t } from '../i18n'

const RELEASES_API = 'https://api.github.com/repos/JanVDB2000/symfo-linker-profiler/releases/latest'
const RELEASES_PAGE = 'https://github.com/JanVDB2000/symfo-linker-profiler/releases/latest'

type Channel = 'managed' | 'appimage' | 'nix' | 'package'

const current = ref('')
const latest = ref('')
const releaseTag = ref('')
const channel = ref<Channel>('package')
const busy = ref(false)
const failed = ref(false)
const checked = ref(false)

/** Compares `1.2.10` against `1.2.9` numerically, so 10 sorts above 9. */
function isNewer(candidate: string, running: string): boolean {
  if (![candidate, running].every(value => /^v?\d+\.\d+\.\d+$/.test(value))) return false
  const parts = (value: string) => value.replace(/^v/, '').split('.').map(Number)
  const [a, b] = [parts(candidate), parts(running)]
  for (let index = 0; index < Math.max(a.length, b.length); index++) {
    const [left, right] = [a[index] ?? 0, b[index] ?? 0]
    // A tag that is not a plain version yields nothing rather than a guess.
    if (Number.isNaN(left) || Number.isNaN(right)) return false
    if (left !== right) return left > right
  }
  return false
}

async function check() {
  if (busy.value) return
  busy.value = true
  failed.value = false
  try {
    if (!isTauri()) throw new Error('the desktop app is required')
    current.value = await getVersion()
    channel.value = (await invoke<string>('update_channel')) as Channel
    const response = await fetch(RELEASES_API, { headers: { Accept: 'application/vnd.github+json' } })
    if (!response.ok) throw new Error(String(response.status))
    const release: unknown = await response.json()
    const tag = typeof release === 'object' && release !== null && 'tag_name' in release
      ? String((release as { tag_name: unknown }).tag_name)
      : ''
    latest.value = isNewer(tag, current.value) ? tag.replace(/^v/, '') : ''
    releaseTag.value = latest.value ? tag : ''
    checked.value = true
  } catch {
    // Deliberately one message for every cause: offline, rate limited, no release
    // published yet. None is actionable differently, and the releases page is the
    // answer to all of them.
    failed.value = true
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <div class="update-check">
    <button class="button" :disabled="busy" @click="check"><Spinner v-if="busy" />{{ busy ? t('Checking…') : t('Check for updates') }}</button>

    <p v-if="failed" class="text-warning">{{ t('The latest version could not be read. Open the releases page instead.') }}</p>

    <template v-else-if="checked">
      <p v-if="!latest" class="muted">{{ t('SymfoLinker {version} is the latest version.', { version: current }) }}</p>
      <template v-else>
        <p><strong>{{ t('SymfoLinker {version} is available.', { version: latest }) }}</strong></p>
        <!-- Each branch says what to actually do, because only one of them is a
             download and none is a button this app can press on your behalf. -->
        <template v-if="channel === 'nix'">
          <p class="help">{{ t('This copy was installed with Nix, which keeps it in a read-only store. Find its entry with nix profile list, remove it with nix profile remove <name>, then install this release:') }}</p>
          <code class="command-output">nix profile install github:JanVDB2000/symfo-linker-profiler/{{ releaseTag }}</code>
        </template>
        <p v-else-if="channel === 'package'" class="help">{{ t('This copy came from a package, so your package manager owns it. Install the new .deb or .rpm from the releases page.') }}</p>
        <p v-else class="help">{{ t('Download the installer for this platform from the releases page.') }}</p>
      </template>
    </template>

    <p class="help"><a :href="RELEASES_PAGE" target="_blank" rel="noreferrer noopener">{{ t('Open the releases page') }}</a></p>
  </div>
</template>
