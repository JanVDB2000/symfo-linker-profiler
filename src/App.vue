<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { isTauri } from '@tauri-apps/api/core'
import { useWorkspace } from './stores/workspace'
import ProfilerIcon from './components/ProfilerIcon.vue'
import Spinner from './components/Spinner.vue'
import type { ArchiveEntry, ExternalTool, PackageStatus } from './types'
import { t, locale, languages, translateMessage } from './i18n'

const workspace = useWorkspace()
const tab = ref('Projects')
// Follow the system theme until the user chooses a preference.
const storedTheme = localStorage.getItem('symfolinker.theme')
const theme = ref(storedTheme === 'light' || storedTheme === 'dark' ? storedTheme : 'auto')
const systemTheme = window.matchMedia('(prefers-color-scheme: dark)')
const systemDark = ref(systemTheme.matches)
const resolvedTheme = computed(() => theme.value === 'auto' ? systemDark.value ? 'dark' : 'light' : theme.value)
function updateSystemTheme(event: MediaQueryListEvent) { systemDark.value = event.matches }
systemTheme.addEventListener('change', updateSystemTheme)
onUnmounted(() => systemTheme.removeEventListener('change', updateSystemTheme))
watch(theme, value => localStorage.setItem('symfolinker.theme', value))
const wide = ref(localStorage.getItem('symfolinker.width') === 'full')
watch(wide, value => localStorage.setItem('symfolinker.width', value ? 'full' : 'normal'))
const workspaceControls = ref(false)
const profileName = ref('')
const pendingMounts = ref(false)
const pendingProfile = ref<(typeof workspace.workspaceProfiles)[number] | null>(null)
function saveProfile() { if (workspace.saveProfile(profileName.value)) profileName.value = '' }
const tabs = [
  { name: 'Projects', icon: 'projects' }, { name: 'Active links', icon: 'links' },
  { name: 'Runtime', icon: 'runtime' }, { name: 'Health', icon: 'health' },
  { name: 'Settings', icon: 'settings' },
]
const ARROW = '→'
const HOOK_ARROW = '↳'
// How long the result of a mount synchronisation stays on screen.
const SUMMARY_VISIBLE_MS = 12000
const localCount = computed(() => workspace.selected?.packages.filter(p => p.mode === 'local').length ?? 0)
const issues = computed(() => workspace.selected?.packages.filter(p => p.mode === 'unknown' || ['interrupted', 'unrecognized', 'lost', 'invalid'].includes(p.backupStatus) || (p.mode === 'local' && p.backupStatus !== 'available')) ?? [])
// Only report a synchronisation that changed something. The common case is a container
// that already sees every local source, and announcing that would be pure noise.
const mountSyncSummary = computed(() => {
  const sync = workspace.mountSync
  if (!sync || (!sync.added.length && !sync.removed.length)) return ''
  return t('Container mounts updated: {added} added, {removed} removed. The PHP service was recreated.',
    { added: sync.added.length, removed: sync.removed.length })
})
// The result has been read by the time the next thought arrives. A banner that stays
// for the rest of the session turns into furniture, and the Mounts table keeps the
// lasting answer anyway.
const summaryVisible = ref(false)
let summaryTimer: ReturnType<typeof setTimeout> | null = null
watch(mountSyncSummary, summary => {
  if (summaryTimer) clearTimeout(summaryTimer)
  summaryVisible.value = !!summary
  if (summary) summaryTimer = setTimeout(() => { summaryVisible.value = false }, SUMMARY_VISIBLE_MS)
})

// One place decides whether switching is possible, and says why not. Every control
// that switches reads this, so a new reason never has to be repeated across the
// template -- and a disabled button can always explain itself.
const switchBlocked = computed(() => {
  if (workspace.busy) return t('The workspace is being scanned.')
  if (workspace.swapping) return t('Another switch is still running.')
  if (workspace.mountSyncBusy) return t('The container mounts are being updated. Switching is possible again in a moment.')
  if (workspace.mutationBlocked) return t('Refresh the workspace before switching again.')
  if (workspace.root !== workspace.result?.developmentRoot) return t('Scan this development root before switching.')
  return ''
})
// Product names stay as they are written; the two generic tools are translated.
const TOOL_LABELS: Record<ExternalTool, string> = {
  phpstorm: 'PhpStorm', vscode: 'VS Code', terminal: 'Terminal', files: 'File manager',
}
function toolLabel(tool: ExternalTool): string {
  return tool === 'terminal' || tool === 'files' ? t(TOOL_LABELS[tool]) : TOOL_LABELS[tool]
}
// Which tools this computer has is asked once, when the interface opens.
void workspace.loadExternalTools()
// The packages whose checkout autoloads differently than the generated autoloader
// expects: the one problem the comparison can report but not solve by itself.
const autoloadDiffers = computed(() => workspace.selected?.packages.filter(pkg => pkg.autoloadStatus === 'differs') ?? [])
// Regenerating reads the whole project and rescans afterwards, so it waits for the
// same quiet workspace a switch does.
const dumpBlocked = computed(() => workspace.autoloadBusy ? t('The autoloader is being regenerated.') : switchBlocked.value)

// Confirmation state for a pending switch; null while no dialog is open.
const pendingSwap = ref<{ pkg: PackageStatus; target: 'local' | 'vendor'; projectId: string } | null>(null)
// Switching back and forth is the rhythm of a working day, so the confirmation can be
// waived. Deliberately not persisted: a new session asks again.
const skipSwapConfirm = ref(false)
function askSwap(pkg: PackageStatus, target: 'local' | 'vendor') {
  if (pkg.mode === target || switchBlocked.value) return
  if (skipSwapConfirm.value) { void workspace.setMode(pkg, target); return }
  pendingSwap.value = { pkg, target, projectId: workspace.selectedId }
}
async function confirmSwap() {
  const pending = pendingSwap.value
  if (!pending || pending.projectId !== workspace.selectedId) return
  pendingSwap.value = null
  await workspace.setMode(pending.pkg, pending.target)
}
function closeDialogs() {
  pendingSwap.value = null
  pendingProfile.value = null
  pendingMounts.value = false
  pendingRelink.value = false
  pendingArchive.value = null
}
// Restoring a set of links and touching a preserved version both move directories, so
// both are confirmed. The archive dialog carries which of the two actions was asked.
const pendingRelink = ref(false)
const pendingArchive = ref<{ entry: ArchiveEntry; action: 'restore' | 'delete' } | null>(null)
const archiveTotal = computed(() => workspace.archives.reduce((total, entry) => total + entry.sizeBytes, 0))
// A preserved version can only take the backup's place while the package is LOCAL; on
// VENDOR the Composer directory is the one in use.
function restorable(entry: ArchiveEntry): boolean {
  return entry.directory && !!workspace.selected?.packages
    .some(pkg => pkg.packageName === entry.packageName && pkg.mode === 'local')
}
const BYTE_STEP = 1024
const BYTE_UNITS = ['B', 'kB', 'MB', 'GB', 'TB']
// Sizes exist to be compared at a glance, which a raw byte count is not.
function formatBytes(bytes: number): string {
  let size = bytes
  let unit = 0
  while (size >= BYTE_STEP && unit < BYTE_UNITS.length - 1) { size /= BYTE_STEP; unit += 1 }
  return `${unit && size < 10 ? size.toFixed(1) : Math.round(size)} ${BYTE_UNITS[unit]}`
}
// Seconds since the epoch, as the backend records them; 0 means none was recoverable.
function formatMoment(seconds: number): string {
  return seconds ? new Date(seconds * 1000).toLocaleString(locale.value) : '—'
}
const linkKindLabel = computed(() => {
  const kind = workspace.preflight?.linkKind
  if (kind === 'symlink') return t('Relative symlink')
  if (kind === 'junction') return t('Junction, with an absolute target')
  return t('No link could be created')
})
// Move focus to the confirming button when the dialog opens, so Enter confirms and a
// keyboard user does not have to tab in from wherever they were.
const confirmButton = ref<HTMLButtonElement | null>(null)
watch(pendingSwap, async pending => {
  if (!pending) return
  await nextTick()
  confirmButton.value?.focus()
})
// A tool that stays open all day earns one shortcut: jump to the filter without
// reaching for the mouse. Escape closes whatever dialog is open, because a modal
// without a keyboard exit is a trap.
const searchInput = ref<HTMLInputElement | null>(null)
function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') { closeDialogs(); return }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault()
    searchInput.value?.select()
  }
}
document.addEventListener('keydown', onKeydown)
onUnmounted(() => {
  document.removeEventListener('keydown', onKeydown)
  if (summaryTimer) clearTimeout(summaryTimer)
})

watch(() => `${workspace.root}\0${workspace.selectedId}`, closeDialogs)

const docker = computed(() => workspace.status?.docker ?? null)
const servicesUp = computed(() => docker.value?.services.filter(s => s.running).length ?? 0)
// Summarize the runtime status, including unavailable and empty states.
const dockerLabel = computed(() => {
  if (workspace.statusError) return { text: t('Status unknown'), cls: 'status-warning' }
  const status = docker.value
  if (!status) return { text: t('Checking...'), cls: '' }
  if (!status.available) return { text: t('Docker unavailable'), cls: 'status-error' }
  if (!status.services.length) return { text: status.composeFile ? t('No containers') : t('Native'), cls: '' }
  const total = status.services.length
  return {
    text: t('{running}/{total} services running', { running: servicesUp.value, total }),
    cls: servicesUp.value === total ? 'status-success' : 'status-warning',
  }
})
const checkedAt = computed(() => workspace.lastChecked ? new Date(workspace.lastChecked).toLocaleTimeString(locale.value) : '')
// A restored scan can be days old, so this one carries the date as well as the time.
const scannedLabel = computed(() => workspace.scannedAt ? new Date(workspace.scannedAt).toLocaleString(locale.value) : '')
const statusTitle = computed(() => workspace.busy ? t('Scanning workspace') : workspace.result ? t('Workspace scanned') : t('No workspace selected'))
const pickerEmpty = computed(() => workspace.search ? t('No projects match this filter.') : workspace.result ? t('No projects found.') : t('Choose a root to get started.'))
// Show incoming links and the target project's own outgoing links.
const usedBy = computed(() => workspace.selected ? workspace.linkedBy.get(workspace.selected.id) ?? [] : [])
function onwardLinks(projectId: string): string[] { return workspace.linksFrom.get(projectId) ?? [] }
const labels: Record<string, string> = {
  notLinked: 'Vendor directory', linked: 'Local source reachable', broken: 'Broken link',
  unexpectedTarget: 'Unexpected link target', missing: 'Vendor package missing', invalid: 'Invalid path',
}
function status(pkg: PackageStatus) { return t(labels[pkg.linkStatus] ?? pkg.linkStatus) }
function count(name: string) {
  if (name === 'Projects') return workspace.result?.projects.length ?? 0
  if (name === 'Active links') return workspace.links.length
  if (name === 'Health') return issues.value.length + (workspace.result?.warnings.length ?? 0)
  return null
}
async function chooseRoot() {
  if (!isTauri()) { await workspace.scan(); return }
  try {
    const directory = await open({ directory: true, multiple: false, title: t('Choose your development root') })
    if (typeof directory === 'string') { workspace.root = directory; await workspace.scan() }
  } catch { workspace.error = { key: 'The directory picker could not be opened. Enter the path manually.' } }
}
</script>

<template>
  <div class="profiler" :class="[`theme-${resolvedTheme}`, { 'width-full': wide }]">
    <div class="profiler-container">
    <header class="profiler-header">
      <a class="brand" href="#" @click.prevent="tab = 'Projects'"><img class="brand-symbol" src="/icon.png" alt="" width="28" height="28" /><span>SymfoLinker Profiler</span></a>
      <div class="header-search"><ProfilerIcon name="search" /><input ref="searchInput" v-model="workspace.search" type="search" :placeholder="t('Search projects')" :aria-label="t('Filter projects')" :title="t('Filter projects (Ctrl+K)')" /></div>
    </header>
      <section class="request-summary" :class="{ 'summary-error': workspace.error }" :aria-label="t('Workspace status')" aria-live="polite">
        <div class="request-title"><span class="request-method">{{ t('SCAN') }}</span><h1>{{ workspace.error ? t('Scan incomplete') : workspace.result?.developmentRoot ?? statusTitle }}</h1></div>
        <div class="request-metadata"><span><b>{{ t('Status') }}</b> <span class="status-code">{{ workspace.error ? t('ERROR') : workspace.result ? 'OK' : t('LOCAL') }}</span> <span class="status-text">{{ workspace.error ? t('Scan incomplete') : statusTitle }}</span> <Spinner v-if="workspace.busy" /></span><span v-if="scannedLabel"><b>{{ t('Scanned') }}</b> {{ scannedLabel }} <span v-if="workspace.fromCache" class="label" :title="t('Shown from the scan saved on this computer. Refresh reads the filesystem again.')">{{ t('Cached') }}</span></span><span><b>{{ t('Project') }}</b> {{ workspace.selected?.name ?? '—' }}</span><span><b>{{ t('Runtime') }}</b> {{ workspace.selected ? workspace.selected.composeFile ? 'Docker Compose' : t('Native') : '—' }}</span><span><b>{{ t('Access') }}</b> {{ t('Read only') }}</span></div>
      </section>
      <div class="profiler-layout">
        <aside class="sidebar">
          <div class="sidebar-contents">
          <div class="sidebar-shortcuts"><button class="text-button" :aria-expanded="workspaceControls || !workspace.result" aria-controls="workspace-controls" @click="workspaceControls = !workspaceControls"><ProfilerIcon name="search" />{{ t('Workspace') }}</button><button class="text-button" :disabled="!!workspace.swapping || workspace.busy || !workspace.root" @click="workspace.scan()"><Spinner v-if="workspace.busy" />{{ workspace.busy ? t('Scanning…') : t('Refresh') }}</button></div>
          <form v-if="workspaceControls || !workspace.result" id="workspace-controls" class="root-form" @submit.prevent="workspace.scan()"><label for="root">{{ t('Development root') }}</label><div class="root-controls"><input id="root" v-model="workspace.root" :placeholder="t('D:\\dev or /home/user/dev')" :disabled="!!workspace.swapping || workspace.busy" /><button type="button" class="button button-small" :disabled="!!workspace.swapping || workspace.busy" @click="chooseRoot"><ProfilerIcon name="folder" />{{ t('Browse') }}</button><button class="button button-small" :disabled="!!workspace.swapping || workspace.busy"><Spinner v-if="workspace.busy" />{{ workspace.busy ? t('Scanning…') : t('Scan') }}</button></div></form>
          <nav class="collector-menu" :aria-label="t('Main navigation')">
            <button v-for="item in tabs" :key="item.name" :class="{ active: tab === item.name }" :aria-current="tab === item.name ? 'page' : undefined" @click="tab = item.name"><ProfilerIcon :name="item.icon" /><span>{{ t(item.name) }}</span><span v-if="count(item.name) !== null" class="menu-count" :class="{ 'count-warning': item.name === 'Health' && count(item.name) }">{{ count(item.name) }}</span></button>
          </nav>
          </div>
          <div class="project-picker">
            <h2>{{ t('Workspace projects') }}</h2>
            <div v-if="workspace.result?.projects.length" class="picker-search"><ProfilerIcon name="search" /><input v-model="workspace.search" type="search" :placeholder="t('Filter projects')" :aria-label="t('Filter workspace projects')" /></div>
            <div class="project-list">
              <template v-for="entry in workspace.projectGroups" :key="entry.group">
                <h3 v-if="entry.group" class="project-group" :title="entry.group">{{ entry.group }}</h3>
                <button v-for="project in entry.projects" :key="project.id" class="project-button" :class="{ selected: project.id === workspace.selectedId }" :aria-pressed="project.id === workspace.selectedId" :title="project.id" @click="workspace.selectedId = project.id; tab = 'Projects'"><ProfilerIcon name="folder" /><span>{{ project.name }}</span><span v-if="project.git?.dirty" class="dirty-dot" role="img" :aria-label="t('Changed files')" :title="t('Changed files')"></span></button>
              </template>
              <p v-if="!workspace.projects.length" class="sidebar-empty">{{ pickerEmpty }}</p>
            </div>
          </div>
          <div class="sidebar-settings"><button class="text-button" @click="tab = 'Settings'"><ProfilerIcon name="settings" />{{ t('Settings') }}</button><button class="theme-toggle" :aria-label="resolvedTheme === 'dark' ? t('Activate light theme') : t('Activate dark theme')" @click="theme = resolvedTheme === 'dark' ? 'light' : 'dark'"><ProfilerIcon :name="resolvedTheme === 'dark' ? 'sun' : 'moon'" /></button><button class="theme-toggle" :aria-pressed="wide" :aria-label="wide ? t('Normal width') : t('Full width')" @click="wide = !wide"><ProfilerIcon :name="wide ? 'collapse' : 'expand'" /></button></div>
        </aside>
        <main class="collector-content">
          <div class="collector-heading"><h2>{{ t(tab) }}</h2></div>
          <div v-if="workspace.swapError" class="notice error" role="alert">{{ translateMessage(workspace.swapError) }}</div>
          <div v-if="workspace.mountSyncBusy" class="notice" role="status"><Spinner />{{ t('Synchronising container mounts. The PHP service is recreated only if they changed.') }}</div>
          <div v-else-if="workspace.mountSyncError" class="notice warning" role="alert">{{ translateMessage(workspace.mountSyncError) }}</div>
          <div v-else-if="summaryVisible" class="notice" role="status">{{ mountSyncSummary }}</div>
          <div v-if="workspace.error" class="notice error" role="alert">{{ translateMessage(workspace.error) }}</div>
          <details v-if="workspace.result?.warnings.length" class="notice warning" open><summary>{{ t('{count} scan issue(s)', { count: workspace.result.warnings.length }) }}</summary><ul><li v-for="warning in workspace.result.warnings" :key="JSON.stringify(warning)">{{ translateMessage(warning) }}</li></ul></details>
          <div v-if="workspace.relinkablePackages.length" class="notice warning" role="status">
            <strong>{{ t('{count} replaced link(s)', { count: workspace.relinkablePackages.length }) }}</strong>
            <p>{{ t('A composer install wrote its own directory over these links. Restoring them switches the packages back to LOCAL; the newly installed version is preserved first, as always.') }}</p>
            <ul><li v-for="entry in workspace.relinkablePackages" :key="`${entry.projectId}/${entry.packageName}`">{{ entry.projectName }} / {{ entry.packageName }}</li></ul>
            <button class="button button-primary" :disabled="!!switchBlocked" :title="switchBlocked || undefined" @click="pendingRelink = true">{{ t('Restore replaced links') }}</button>
          </div>
          <template v-if="tab === 'Projects'">
            <div class="metrics"><div class="metric"><strong>{{ workspace.result?.projects.length ?? '—' }}</strong><span>{{ t('Projects') }}</span></div><div class="metric"><strong>{{ workspace.selected?.packages.length ?? '—' }}</strong><span>{{ t('Local dependencies') }}</span></div><div class="metric"><strong>{{ workspace.selected ? localCount : '—' }}</strong><span>{{ t('Active links') }}</span></div><div class="metric"><strong :class="{ 'text-warning': issues.length }">{{ workspace.selected ? issues.length : '—' }}</strong><span>{{ t('Issues') }}</span></div></div>
            <div v-if="workspace.busy && !workspace.result" class="empty empty-panel scanning-panel"><Spinner size="large" :label="t('Scanning workspace')" /><h3>{{ t('Scanning workspace') }}</h3><p>{{ t('Reading composer.json, Git status and vendor paths.') }}</p></div>
            <template v-else-if="workspace.selected">
              <h3 class="heading-live">{{ t('Project details') }} <small>{{ workspace.selected.name }}</small><span class="heading-actions"><span v-if="checkedAt" class="live" :title="workspace.watching ? t('A composer install or a branch change under this project is noticed as it happens.') : t('Sync every 5 minutes while this project is selected')"><span class="live-dot"></span>{{ t('updated {time}', { time: checkedAt }) }}<span v-if="workspace.watching" class="label">{{ t('watching') }}</span></span><button class="button button-small" :disabled="workspace.statusBusy || workspace.busy || !!workspace.swapping" :title="t('Refresh Git and container status for {project} now', { project: workspace.selected.name })" @click="workspace.scan()"><ProfilerIcon name="refresh" />{{ (workspace.statusBusy || workspace.busy) ? t('Syncing...') : t('Sync') }}</button></span></h3>
              <div class="table-scroll"><table class="property-table"><tbody>
                <tr><th scope="row">{{ t('Composer package') }}</th><td>{{ workspace.selected.composerName ?? t('Not configured') }}</td></tr>
                <tr v-if="workspace.selected.group"><th scope="row">{{ t('Group') }}</th><td>{{ workspace.selected.group }}</td></tr>
                <tr><th scope="row">{{ t('Project path') }}</th><td class="path">{{ workspace.selected.path }}</td></tr>
                <tr><th scope="row">{{ t('Git branch') }}</th><td><span class="branch"><ProfilerIcon name="branch" />{{ workspace.selected.git?.branch ?? t('No Git information') }}</span><code v-if="workspace.selected.git?.commit" class="commit">{{ workspace.selected.git.commit }}</code></td></tr>
                <tr><th scope="row">{{ t('Working directory') }}</th><td><span v-if="workspace.selected.git" class="label" :class="workspace.selected.git.dirty ? 'status-warning' : 'status-success'">{{ workspace.selected.git.dirty ? t('{count} changed files', { count: workspace.selected.git.changedFiles }) : t('Clean') }}</span><span v-else class="muted">{{ t('Unavailable') }}</span></td></tr>
                <tr><th scope="row">{{ t('Containers') }}</th><td><span class="label" :class="dockerLabel.cls">{{ dockerLabel.text }}</span><span v-if="docker?.message" class="muted docker-note">{{ translateMessage(docker.message) }}</span><span v-else-if="workspace.statusError" class="muted docker-note">{{ translateMessage(workspace.statusError) }}</span></td></tr>
                <tr><th scope="row">{{ t('Open in') }}</th><td><div class="open-in">
                  <button v-for="tool in workspace.externalTools" :key="tool" class="button button-small" :title="t('Open {project} in {tool}', { project: workspace.selected.name, tool: toolLabel(tool) })" @click="workspace.openExternal(tool)"><ProfilerIcon name="external" />{{ toolLabel(tool) }}</button>
                  <button class="button button-small" :title="t('Copy the project path to the clipboard')" @click="workspace.copyPath(workspace.selected.path)"><ProfilerIcon name="copy" />{{ workspace.copiedPath === workspace.selected.path ? t('Path copied') : t('Copy path') }}</button>
                  <span v-if="!workspace.externalTools.length" class="muted">{{ t('No editor, terminal or file manager was found on PATH.') }}</span>
                </div><p v-if="workspace.externalError" class="text-warning open-in-error">{{ translateMessage(workspace.externalError) }}</p></td></tr>
                <tr><th scope="row">{{ t('Used by') }}</th><td><template v-if="usedBy.length"><button v-for="name in usedBy" :key="name" class="text-button chain-link" @click="workspace.selectedId = name; tab = 'Projects'">{{ name }}</button></template><span v-else class="muted">{{ t('No project links to this project') }}</span></td></tr>
              </tbody></table></div>
              <h3 class="heading-live">{{ t('Local dependencies') }} <small>{{ workspace.selected.packages.length }}</small>
                <span class="heading-actions">
                  <span v-if="workspace.mountSyncBusy" class="mount-sync"><Spinner />{{ t('Updating container mounts…') }}</span>
                  <button v-if="workspace.lastSwitch" class="button button-small" :disabled="!!switchBlocked" :title="switchBlocked || t('Put back what the last switch moved: {name}', { name: workspace.lastSwitch.label })" @click="workspace.undoSwitch()"><ProfilerIcon name="refresh" />{{ t('Undo last switch') }}</button>
                </span>
              </h3>
              <div v-if="autoloadDiffers.length" class="notice warning" role="status">
                <strong>{{ t('{count} package(s) autoload differently', { count: autoloadDiffers.length }) }}</strong>
                <p>{{ t('The checkout declares different autoload rules than the generated autoloader was built from, so classes will not be found. Regenerating runs composer dump-autoload in the PHP container when it is running, and on this computer otherwise.') }}</p>
                <ul><li v-for="pkg in autoloadDiffers" :key="pkg.packageName">{{ pkg.packageName }}</li></ul>
                <button class="button button-primary" :disabled="!!dumpBlocked" :title="dumpBlocked || undefined" @click="workspace.dumpAutoload()"><Spinner v-if="workspace.autoloadBusy" /><ProfilerIcon v-else name="refresh" />{{ workspace.autoloadBusy ? t('Regenerating the autoloader...') : t('Regenerate the autoloader') }}</button>
              </div>
              <div v-if="workspace.autoloadError" class="notice error" role="alert">{{ translateMessage(workspace.autoloadError) }}</div>
              <details v-else-if="workspace.autoloadDump" class="notice">
                <summary>{{ workspace.autoloadDump.location === 'container' ? t('Autoloader regenerated in {service}', { service: workspace.autoloadDump.service ?? '' }) : t('Autoloader regenerated on this computer') }}</summary>
                <p v-if="workspace.autoloadDump.message" class="muted">{{ translateMessage(workspace.autoloadDump.message) }}</p>
                <pre class="command-output">{{ workspace.autoloadDump.output || t('Composer reported nothing.') }}</pre>
              </details>
              <div v-if="!workspace.selected.packages.length" class="empty"><p>{{ t('No dependencies with a unique local Composer package found.') }}</p></div>
              <div v-else class="table-scroll"><table class="packages-table"><thead><tr><th scope="col">{{ t('Package / constraint') }}</th><th scope="col">{{ t('Mode') }}</th><th scope="col">{{ t('Git branch') }}</th><th scope="col">{{ t('Host status') }}</th><th scope="col">{{ t('Backup') }}</th></tr></thead><tbody>
                <tr v-for="pkg in workspace.selected.packages" :key="`${pkg.packageName}-${pkg.dependencyType}`">
                  <td><strong class="package-name">{{ pkg.packageName }}</strong><div class="package-constraint">{{ pkg.constraint }} <span v-if="pkg.dependencyType === 'requireDev'" class="label">require-dev</span><span v-if="pkg.localVersion" class="muted"> {{ t('checkout provides {version}', { version: pkg.localVersion }) }}</span></div>
                    <div class="package-flags">
                      <span v-if="pkg.relinkable" class="label status-warning" :title="t('SymfoLinker recorded a link here, and a Composer directory took its place. Restore it from the notice above the panel.')">{{ t('Link replaced') }}</span>
                      <span v-if="pkg.constraintStatus === 'mismatch'" class="label status-warning" :title="t('The checkout provides {version}, which this project does not ask for ({constraint}). A link bypasses Composer, so nothing refused it.', { version: pkg.localVersion ?? '', constraint: pkg.constraint })">{{ t('Version conflict') }}</span>
                      <span v-if="pkg.autoloadStatus === 'differs'" class="label status-warning" :title="t('The checkout declares different autoload rules than the generated autoloader uses. Run composer dump-autoload in this project after switching, or classes will not be found.')">{{ t('Autoload differs') }}</span>
                    </div><details class="package-paths"><summary>{{ t('Paths') }}</summary><dl><dt>{{ t('Local') }}</dt><dd>{{ pkg.localPath }}</dd><dt>{{ t('Vendor') }}</dt><dd>{{ pkg.vendorPath }}</dd><dt>{{ t('Backup') }}</dt><dd>{{ pkg.backupPath }}</dd></dl></details></td>
                  <td><div v-if="pkg.mode === 'unknown'" class="mode-switch"><span class="label status-warning">{{ t('UNKNOWN') }}</span></div>
                    <div v-else class="mode-switch" role="group" :aria-label="t('Mode for {name}', { name: pkg.packageName })">
                      <button class="mode-option" :class="{ active: pkg.mode === 'vendor' }" :aria-pressed="pkg.mode === 'vendor'" :disabled="!!switchBlocked" :title="switchBlocked || undefined" @click="askSwap(pkg, 'vendor')">{{ t('VENDOR') }}</button>
                      <button class="mode-option" :class="{ active: pkg.mode === 'local' }" :aria-pressed="pkg.mode === 'local'" :disabled="!!switchBlocked" :title="switchBlocked || undefined" @click="askSwap(pkg, 'local')">{{ t('LOCAL') }}</button>
                      <span v-if="workspace.swapping === pkg.packageName" class="mode-busy">{{ t('Switching...') }}</span>
                    </div></td>
                  <td><span class="branch"><ProfilerIcon name="branch" />{{ pkg.git?.branch ?? '—' }}</span><div v-if="pkg.git?.dirty" class="text-warning">{{ t('{count} changed', { count: pkg.git.changedFiles }) }}</div></td>
                  <td :class="pkg.mode === 'unknown' ? 'text-warning' : 'text-success'">{{ status(pkg) }}</td>
                  <td :class="{ 'text-warning': pkg.mode === 'local' && pkg.backupStatus !== 'available' }">{{ t({ missing: 'Missing', available: 'Available', invalid: 'Invalid', interrupted: 'Interrupted', unrecognized: 'Unrecognized', lost: 'Lost' }[pkg.backupStatus]) }}</td>
                </tr>
              </tbody></table></div>
            </template>
            <div v-else class="empty empty-panel"><ProfilerIcon name="projects" /><h3>{{ workspace.result ? t('No Composer projects found') : t('Select your development root') }}</h3><p>{{ workspace.result ? t('The scanner looks for composer.json up to four directories below the root, so projects grouped in folders are found where they are. Choose the directory that holds your projects.') : t('Choose the directory containing your Composer projects. Their local dependencies, Git status and vendor paths will appear here.') }}</p><button class="button" :disabled="!!workspace.swapping || workspace.busy" @click="chooseRoot">{{ t('Choose development root') }}</button></div>
          </template>
          <template v-else-if="tab === 'Active links'">
            <div class="metrics"><div class="metric"><strong>{{ workspace.linkedPackages.length }}</strong><span>{{ t('Linked packages') }}</span></div><div class="metric"><strong>{{ workspace.links.length }}</strong><span>{{ t('Active local links') }}</span></div></div>
            <h3>{{ t('Links across all projects') }}</h3>
            <div v-if="!workspace.linkedPackages.length" class="empty"><p>{{ workspace.result ? t('No active local links found.') : t('Scan your workspace to view active links.') }}</p></div>
            <div v-for="entry in workspace.linkedPackages" :key="entry.packageName" class="link-group">
              <div class="link-head">
                <strong class="package-name">{{ entry.packageName }}</strong>
                <span class="label status-success">{{ t('LOCAL') }}</span>
                <span class="branch"><ProfilerIcon name="branch" />{{ entry.git?.branch ?? t('No Git information') }}</span>
                <span v-if="entry.git?.dirty" class="label status-warning">{{ t('{count} changed files', { count: entry.git.changedFiles }) }}</span>
                <button v-if="onwardLinks(entry.localProjectId).length" class="text-button chain-link" :title="t('{project} also links to {packages}', { project: entry.localProjectId, packages: onwardLinks(entry.localProjectId).join(', ') })" @click="workspace.selectedId = entry.localProjectId; tab = 'Projects'">{{ HOOK_ARROW }} {{ onwardLinks(entry.localProjectId).join(', ') }}</button>
              </div>
              <p class="path link-source">{{ ARROW }} {{ entry.localPath }}</p>
              <p class="link-usage">{{ t('Used by {count} project(s)', { count: entry.usedBy.length }) }}</p>
              <div class="link-consumers">
                <button v-for="consumer in entry.usedBy" :key="consumer.id" class="text-button" @click="workspace.selectedId = consumer.id; tab = 'Projects'"><ProfilerIcon name="projects" />{{ consumer.name }}</button>
              </div>
            </div>
          </template>
          <template v-else-if="tab === 'Runtime'">
            <h3>{{ t('Runtime information') }} <small>{{ workspace.selected?.name }}</small></h3>
            <template v-if="workspace.selected">
              <div class="table-scroll"><table class="property-table"><tbody><tr><th scope="row">{{ t('Runtime') }}</th><td>{{ workspace.selected.composeFile ? 'Docker Compose' : t('Native') }}</td></tr><tr><th scope="row">{{ t('Compose file') }}</th><td>{{ workspace.selected.composeFile ?? t('Not found') }}</td></tr><tr><th scope="row">{{ t('Host project') }}</th><td class="path">{{ workspace.selected.path }}</td></tr><tr><th scope="row">{{ t('Containers') }}</th><td><span class="label" :class="dockerLabel.cls">{{ dockerLabel.text }}</span><span v-if="checkedAt" class="muted docker-note">{{ t('updated {time}', { time: checkedAt }) }}</span></td></tr></tbody></table></div>
              <h3 class="heading-live">{{ t('Container') }}
                <span class="heading-actions"><button class="button button-small" :disabled="workspace.containerBusy" @click="workspace.inspectContainer()"><ProfilerIcon name="refresh" />{{ workspace.containerBusy ? t('Inspecting...') : t('Inspect container') }}</button></span>
              </h3>
              <div v-if="workspace.containerError" class="notice error" role="alert">{{ translateMessage(workspace.containerError) }}</div>
              <div v-if="!workspace.container" class="empty"><p>{{ t('Inspect the container to map host paths and validate linked packages.') }}</p></div>
              <template v-else>
                <div v-if="workspace.container.message" class="notice warning">{{ translateMessage(workspace.container.message) }}</div>
                <div class="table-scroll"><table class="property-table"><tbody>
                  <tr><th scope="row">{{ t('PHP service') }}</th><td>
                    <select v-if="workspace.container.services.length" class="service-select" :disabled="workspace.containerBusy" :value="workspace.container.phpService ?? ''" @change="workspace.selectPhpService(($event.target as HTMLSelectElement).value)">
                      <option value="">{{ t('Choose a service') }}</option>
                      <option v-for="name in workspace.container.services" :key="name" :value="name">{{ name }}</option>
                    </select>
                    <span v-else class="muted">{{ t('No services found') }}</span>
                    <span v-if="workspace.container.suggested && workspace.container.phpService" class="label suggested-label">{{ t('suggested') }}</span>
                  </td></tr>
                  <tr><th scope="row">{{ t('Project in container') }}</th><td class="path">{{ workspace.container.projectContainerPath ?? t('Not mapped') }}</td></tr>
                </tbody></table></div>
                <h3>{{ t('Mounts') }}</h3>
                <div v-if="workspace.container.volumes.length" class="table-scroll"><table><thead><tr><th scope="col">{{ t('Host') }}</th><th scope="col">{{ t('Container') }}</th></tr></thead><tbody>
                  <tr v-for="[host, inside] in workspace.container.volumes" :key="host"><td class="path">{{ host }}</td><td class="path">{{ inside }}</td></tr>
                </tbody></table></div>
                <div v-else class="empty"><p>{{ t('This service has no bind mounts.') }}</p></div>
                <template v-if="workspace.container.mountPlan?.length">
                  <h3>{{ t('Missing local source mounts') }}</h3>
                  <p class="notice warning">{{ t('The host symlink points outside the application mount. Mount its local source at the destination below.') }}</p>
                  <p class="help">{{ t('Switching a package normally mounts and unmounts its source by itself. These remain because that pass could not change the container.') }}</p>
                  <div class="table-scroll"><table><thead><tr><th>{{ t('Local source:') }}</th><th>{{ t('Container') }}</th></tr></thead><tbody>
                    <tr v-for="mount in workspace.container.mountPlan" :key="mount.containerPath"><td class="path">{{ mount.hostPath }}</td><td class="path">{{ mount.containerPath }}</td></tr>
                  </tbody></table></div>
                  <button class="button button-primary" :disabled="!workspace.container.canApplyMounts || workspace.containerBusy || !!workspace.swapping" @click="pendingMounts = true">{{ t('Apply mounts and recreate PHP service') }}</button>
                  <p class="help">{{ t('The original Compose files stay unchanged. SymfoLinker adds an override and recreates only the selected PHP service after confirmation.') }}</p>
                  <p v-if="!workspace.container.canApplyMounts" class="notice warning">{{ t("The running service's Compose files could not be verified. No containers were changed.") }}</p>
                </template>
                <h3>{{ t('Linked packages in container') }}</h3>
                <div v-if="workspace.container.checks.length" class="table-scroll"><table><thead><tr><th scope="col">{{ t('Package') }}</th><th scope="col">{{ t('Container path') }}</th><th scope="col">{{ t('Present') }}</th><th scope="col">{{ t('Link target') }}</th></tr></thead><tbody>
                  <tr v-for="check in workspace.container.checks" :key="check.packageName">
                    <td><strong class="package-name">{{ check.packageName }}</strong></td>
                    <td class="path">{{ check.mapped ? check.containerPath : t('Not mapped') }}</td>
                    <td><span class="label" :class="check.exists ? 'status-success' : 'status-error'">{{ check.exists ? t('Yes') : t('No') }}</span></td>
                    <td class="path">{{ check.linkTarget ?? '-' }}</td>
                  </tr>
                </tbody></table></div>
                <div v-else class="empty"><p>{{ t('No locally linked packages to validate.') }}</p></div>
              </template>
              <h3>{{ t('Services') }} <small v-if="docker?.services.length">{{ t('{running}/{total} running', { running: servicesUp, total: docker.services.length }) }}</small></h3>
              <div v-if="docker?.services.length" class="table-scroll"><table><thead><tr><th scope="col">{{ t('Service') }}</th><th scope="col">{{ t('Container') }}</th><th scope="col">{{ t('Status') }}</th><th scope="col">{{ t('Health') }}</th><th scope="col">{{ t('Ports') }}</th></tr></thead><tbody>
                <tr v-for="item in docker.services" :key="item.name">
                  <td><strong class="package-name">{{ item.service }}</strong></td>
                  <td class="path">{{ item.name }}</td>
                  <td><span class="label" :class="item.running ? 'status-success' : 'status-error'">{{ t(item.state).toUpperCase() }}</span><div class="package-constraint">{{ item.status }}</div></td>
                  <td :class="item.health === 'unhealthy' ? 'text-warning' : ''">{{ item.health ? t(item.health) : '-' }}</td>
                  <td class="path">{{ item.ports || '-' }}</td>
                </tr>
              </tbody></table></div>
              <div v-else class="notice warning"><strong>{{ dockerLabel.text }}</strong><span v-if="docker?.message"> - {{ translateMessage(docker.message) }}</span><span v-else-if="workspace.statusError"> - {{ translateMessage(workspace.statusError) }}</span></div>
            </template>
            <div v-else class="empty"><p>{{ t('Scan your workspace and select a project.') }}</p></div><p class="help">{{ t('Status comes from docker compose ps in the project directory and refreshes automatically. Mounts follow every switch by themselves; Inspect container re-reads them and validates each linked package inside the container.') }}</p>
          </template>
          <template v-else-if="tab === 'Health'">
            <p class="help">{{ t('Recovery restores preserved packages at empty vendor paths and reconciles completed operations. Occupied paths are left untouched.') }}</p>
            <button class="button" :disabled="!workspace.selected || workspace.busy || !!workspace.swapping || !!workspace.mountSyncBusy" @click="workspace.recoverProject()">{{ t('Recover interrupted swaps') }}</button>
            <h3>{{ t('Write checks') }}</h3>
            <p class="help">{{ t('A switch moves directories and creates a link. These checks write a probe wherever it would, then remove it again, so a missing permission is reported before a switch instead of halfway through one.') }}</p>
            <button class="button" :disabled="!workspace.selected || workspace.preflightBusy || !!switchBlocked" :title="switchBlocked || undefined" @click="workspace.runPreflight()"><Spinner v-if="workspace.preflightBusy" />{{ workspace.preflightBusy ? t('Checking...') : t('Run write checks') }}</button>
            <div v-if="workspace.preflightError" class="notice error" role="alert">{{ translateMessage(workspace.preflightError) }}</div>
            <template v-if="workspace.preflight">
              <div class="table-scroll"><table class="property-table"><tbody>
                <tr><th scope="row">{{ t('Development root') }}</th><td><span class="label" :class="workspace.preflight.rootWritable ? 'status-success' : 'status-error'">{{ workspace.preflight.rootWritable ? t('Writable') : t('Not writable') }}</span></td></tr>
                <tr><th scope="row">{{ t('Vendor directory') }}</th><td><span class="label" :class="workspace.preflight.vendorWritable ? 'status-success' : 'status-error'">{{ workspace.preflight.vendorWritable ? t('Writable') : t('Not writable') }}</span></td></tr>
                <tr><th scope="row">{{ t('Link type') }}</th><td><span class="label" :class="workspace.preflight.linkKind ? 'status-success' : 'status-error'">{{ linkKindLabel }}</span><span v-if="workspace.preflight.linkKind === 'junction'" class="muted docker-note">{{ t('Container path mapping cannot be assumed to work with an absolute target.') }}</span></td></tr>
                <tr><th scope="row">{{ t('Free space') }}</th><td>{{ workspace.preflight.freeBytes === null ? t('Unavailable') : formatBytes(workspace.preflight.freeBytes) }}</td></tr>
                <tr><th scope="row">{{ t('Preserved versions') }}</th><td>{{ formatBytes(workspace.preflight.preservedBytes) }}</td></tr>
              </tbody></table></div>
              <p v-if="workspace.preflight.message" class="notice warning">{{ translateMessage(workspace.preflight.message) }}</p>
            </template>
            <h3>{{ t('Preserved versions') }} <small v-if="workspace.archives.length">{{ formatBytes(archiveTotal) }}</small></h3>
            <p class="help">{{ t('Every Composer version a switch set aside is kept here, and nothing removes it automatically. This is where it is removed by hand, or put back in place of the current one.') }}</p>
            <button class="button" :disabled="!workspace.selected || workspace.archivesBusy" @click="workspace.loadArchives()"><Spinner v-if="workspace.archivesBusy" />{{ workspace.archivesBusy ? t('Reading…') : t('Show preserved versions') }}</button>
            <div v-if="workspace.archivesError" class="notice error" role="alert">{{ translateMessage(workspace.archivesError) }}</div>
            <div v-if="workspace.archives.length" class="table-scroll"><table><thead><tr><th scope="col">{{ t('Package') }}</th><th scope="col">{{ t('Version') }}</th><th scope="col">{{ t('Preserved on') }}</th><th scope="col">{{ t('Size') }}</th><th scope="col">{{ t('Actions') }}</th></tr></thead><tbody>
              <tr v-for="entry in workspace.archives" :key="`${entry.packageName}/${entry.archiveId}`">
                <td><strong class="package-name">{{ entry.packageName }}</strong><div v-if="!entry.recognized" class="package-constraint text-warning">{{ t('Unrecognized') }}</div></td>
                <td>{{ entry.version ?? '—' }}</td>
                <td>{{ formatMoment(entry.archivedAt) }}</td>
                <td>{{ entry.directory ? `${formatBytes(entry.sizeBytes)} · ${t('{count} files', { count: entry.files })}` : t('Not a package directory') }}</td>
                <td><div class="row-actions">
                  <button class="button button-small" :disabled="!restorable(entry) || workspace.archivesBusy || !!switchBlocked" :title="restorable(entry) ? switchBlocked || t('Make this the version a switch to VENDOR restores') : t('Switch this package to LOCAL first: on VENDOR the Composer directory is the one in use.')" @click="pendingArchive = { entry, action: 'restore' }">{{ t('Restore') }}</button>
                  <button class="button button-small" :disabled="workspace.archivesBusy || !!switchBlocked" :title="switchBlocked || t('Remove this preserved version for good')" @click="pendingArchive = { entry, action: 'delete' }">{{ t('Remove') }}</button>
                </div></td>
              </tr>
            </tbody></table></div>
            <div v-else-if="!workspace.archivesBusy && !workspace.archivesError" class="empty"><p>{{ t('No preserved versions were read for this project yet.') }}</p></div>
            <h3>{{ t('Environment') }}</h3><div class="table-scroll"><table class="property-table"><tbody><tr><th scope="row">{{ t('Development root') }}</th><td><span class="label" :class="workspace.result ? 'status-success' : ''">{{ workspace.result ? t('Readable') : t('Not scanned yet') }}</span></td></tr><tr><th scope="row">{{ t('Composer projects') }}</th><td>{{ workspace.result?.projects.length ?? '—' }}</td></tr><tr><th scope="row">{{ t('Change detection') }}</th><td><span class="label" :class="workspace.watching ? 'status-success' : ''">{{ workspace.watching ? t('Watching the filesystem') : t('Polling every 5 minutes') }}</span></td></tr>
                      <tr><th scope="row">{{ t('Git information') }}</th><td>{{ workspace.selected?.git ? t('Available') : t('Unavailable') }}</td></tr><tr><th scope="row">{{ t('Package issues') }}</th><td>{{ workspace.selected ? issues.length : '—' }}</td></tr></tbody></table></div>
            <h3>{{ t('Packages') }} <small>{{ workspace.selected?.name }}</small></h3><div v-for="pkg in issues" :key="`${pkg.packageName}-${pkg.dependencyType}`" class="notice warning"><strong>{{ pkg.packageName }}</strong> — {{ pkg.mode === 'local' ? t('Local link has no valid vendor backup.') : status(pkg) }}</div><div v-if="!issues.length" class="empty"><p>{{ workspace.selected ? t('No package issues found.') : t('Select a scanned project to check its packages.') }}</p></div><p class="help">{{ t('This check reports readability and path status. Container mounts and linked packages are checked under Runtime.') }}</p>
          </template>
          <template v-else-if="tab === 'Settings'">
            <h3>{{ t('Saved workspaces') }}</h3>
            <div v-for="item in workspace.savedWorkspaces" :key="item.root" class="row-actions saved-entry">
              <button class="text-button path" :disabled="workspace.busy || !!workspace.swapping" @click="workspace.openWorkspace(item.root)">{{ item.name }}</button>
              <button class="button button-small" :disabled="workspace.busy || !!workspace.swapping" @click="workspace.removeWorkspace(item.root)">{{ t('Remove') }}</button>
            </div>
            <h3>{{ t('Link profiles') }}</h3>
            <p>{{ t('Save the current package modes for this workspace. Refresh cached data before saving a profile.') }}</p>
            <form class="row-actions" @submit.prevent="saveProfile">
              <input v-model="profileName" :aria-label="t('Profile name')" :placeholder="t('Profile name')" maxlength="100" />
              <button class="button" :disabled="!profileName.trim() || !workspace.canSaveProfile" :title="workspace.profileBlocked ? translateMessage(workspace.profileBlocked) : undefined">{{ t('Save profile') }}</button>
            </form>
            <p v-if="workspace.profileBlocked" class="help">{{ translateMessage(workspace.profileBlocked) }}</p>
            <div v-for="profile in workspace.workspaceProfiles" :key="profile.name" class="row-actions saved-entry">
              <strong>{{ profile.name }}</strong><span>{{ t('{count} packages', { count: profile.entries.length }) }}</span>
              <button class="button button-small" :disabled="workspace.busy || !!workspace.swapping" @click="pendingProfile = profile">{{ t('Apply profile') }}</button>
              <button class="button button-small" :disabled="workspace.busy || !!workspace.swapping" @click="workspace.removeProfile(profile.name)">{{ t('Remove') }}</button>
            </div>
            <h3>{{ t('Appearance') }}</h3><div class="table-scroll"><table class="property-table"><tbody><tr><th scope="row"><label for="language">{{ t('Language') }}</label></th><td><select id="language" v-model="locale"><option v-for="language in languages" :key="language.code" :value="language.code">{{ language.label }}</option></select></td></tr><tr><th scope="row">{{ t('Theme') }}</th><td><div class="theme-options" role="group" :aria-label="t('Color theme')"><button class="button" :aria-pressed="theme === 'auto'" @click="theme = 'auto'">{{ t('Automatic') }}</button><button class="button" :aria-pressed="theme === 'light'" @click="theme = 'light'"><ProfilerIcon name="sun" />{{ t('Light') }}</button><button class="button" :aria-pressed="theme === 'dark'" @click="theme = 'dark'"><ProfilerIcon name="moon" />{{ t('Dark') }}</button></div></td></tr><tr><th scope="row">{{ t('Scan scope') }}</th><td>{{ t('Immediate subdirectories of the development root') }}</td></tr><tr><th scope="row">{{ t('Project access') }}</th><td><span class="label status-success">{{ t('Read only') }}</span></td></tr><tr><th scope="row">{{ t('Saved scan') }}</th><td><div class="row-actions"><span class="label" :class="workspace.scannedAt ? 'status-success' : ''">{{ workspace.scannedAt ? t('Saved on this computer') : t('Nothing saved') }}</span><span v-if="scannedLabel" class="muted">{{ scannedLabel }}</span><button class="button button-small" :disabled="!workspace.scannedAt" @click="workspace.forgetScan()">{{ t('Clear saved scan') }}</button></div></td></tr></tbody></table></div><h3>{{ t('Workspace') }}</h3><p>{{ t('Use Workspace in the sidebar to change the development root. The last scanned root, theme and language are saved locally.') }}</p><p>{{ t('The scan itself is saved too, so reopening the app shows the same workspace without scanning again. Refresh reads the filesystem anew.') }}</p><p class="help">{{ t('This version reads Composer files, Git status and vendor paths. The scanner does not modify project files.') }}</p>
          </template>
        </main>
      </div>
      <div v-if="pendingSwap" class="dialog-backdrop" @click.self="pendingSwap = null">
        <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="swap-title">
          <h3 id="swap-title">{{ pendingSwap.target === 'local' ? t('Use local package?') : t('Restore vendor package?') }}</h3>
          <p class="dialog-package">{{ pendingSwap.pkg.packageName }}</p>
          <template v-if="pendingSwap.target === 'local'">
            <p>{{ t('The original Composer package will be preserved in:') }}</p>
            <p class="path dialog-path">{{ pendingSwap.pkg.backupPath }}</p>
            <p v-if="pendingSwap.pkg.backupStatus !== 'missing'" class="notice warning">{{ t('Whatever is at the backup location is moved to history first; nothing is deleted. The current Composer package becomes the new backup.') }}</p>
            <p>{{ t('Local source:') }}</p>
            <p class="path dialog-path">{{ pendingSwap.pkg.localPath }}</p>
          </template>
          <p v-else>{{ t('The local link will be removed and the preserved Composer package restored.') }}</p>
          <label class="dialog-skip"><input v-model="skipSwapConfirm" type="checkbox" />{{ t('Switch without asking until the app is closed') }}</label>
          <div class="dialog-actions">
            <button class="button" @click="pendingSwap = null">{{ t('Cancel') }}</button>
            <button ref="confirmButton" class="button button-primary" @click="confirmSwap">{{ pendingSwap.target === 'local' ? t('Use Local') : t('Restore Vendor') }}</button>
          </div>
        </div>
      </div>
      <div v-if="pendingProfile" class="dialog-backdrop" @click.self="pendingProfile = null">
        <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="profile-title">
          <h3 id="profile-title">{{ t('Apply profile') }}: {{ pendingProfile.name }}</h3>
          <p>{{ t('All packages are checked before switching. Completed changes are rolled back if a later switch fails.') }}</p>
          <ul><li v-for="entry in pendingProfile.entries" :key="`${entry.projectId}/${entry.packageName}`">{{ entry.projectId }} / {{ entry.packageName }}: {{ entry.mode.toUpperCase() }}</li></ul>
          <div class="dialog-actions"><button class="button" @click="pendingProfile = null">{{ t('Cancel') }}</button><button class="button button-primary" @click="workspace.applyProfile(pendingProfile); pendingProfile = null">{{ t('Apply profile') }}</button></div>
        </div>
      </div>
      <div v-if="pendingMounts && workspace.container" class="dialog-backdrop" @click.self="pendingMounts = false">
        <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="mount-title">
          <h3 id="mount-title">{{ t('Apply mounts and recreate PHP service') }}</h3>
          <p>{{ t('The PHP service {service} will briefly stop and be recreated with the missing mounts. Other services are not recreated.', { service: workspace.container.phpService ?? '' }) }}</p>
          <p>{{ t('Mounted data is reused. Changes stored only inside the container are lost when it is recreated.') }}</p>
          <ul><li v-for="mount in workspace.container.mountPlan" :key="mount.containerPath" class="path">{{ mount.hostPath }} &rarr; {{ mount.containerPath }}</li></ul>
          <div class="dialog-actions"><button class="button" @click="pendingMounts = false">{{ t('Cancel') }}</button><button class="button button-primary" @click="pendingMounts = false; workspace.applyContainerMounts()">{{ t('Apply mounts and recreate PHP service') }}</button></div>
        </div>
      </div>
      <div v-if="pendingRelink" class="dialog-backdrop" @click.self="pendingRelink = false">
        <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="relink-title">
          <h3 id="relink-title">{{ t('Restore replaced links') }}</h3>
          <p>{{ t('These packages switch back to LOCAL. The version Composer installed is preserved first, so both remain available.') }}</p>
          <ul><li v-for="entry in workspace.relinkablePackages" :key="`${entry.projectId}/${entry.packageName}`">{{ entry.projectName }} / {{ entry.packageName }}</li></ul>
          <p>{{ t('All packages are checked before switching. Completed changes are rolled back if a later switch fails.') }}</p>
          <div class="dialog-actions"><button class="button" @click="pendingRelink = false">{{ t('Cancel') }}</button><button class="button button-primary" @click="pendingRelink = false; workspace.relinkPackages()">{{ t('Restore replaced links') }}</button></div>
        </div>
      </div>
      <div v-if="pendingArchive" class="dialog-backdrop" @click.self="pendingArchive = null">
        <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="archive-title">
          <h3 id="archive-title">{{ pendingArchive.action === 'restore' ? t('Restore this preserved version?') : t('Remove this preserved version?') }}</h3>
          <p class="dialog-package">{{ pendingArchive.entry.packageName }}<span v-if="pendingArchive.entry.version"> · {{ pendingArchive.entry.version }}</span></p>
          <p class="path dialog-path">{{ pendingArchive.entry.path }}</p>
          <template v-if="pendingArchive.action === 'restore'">
            <p>{{ t('This version takes the place of the one preserved now, which is kept in history in turn. The link stays as it is; only what a switch to VENDOR restores changes.') }}</p>
          </template>
          <template v-else>
            <p class="notice warning">{{ t('This deletes the directory for good. It is the only thing in SymfoLinker that deletes anything, and it cannot be undone.') }}</p>
            <p>{{ t('The package in use and the version preserved now are not touched.') }}</p>
          </template>
          <div class="dialog-actions">
            <button class="button" @click="pendingArchive = null">{{ t('Cancel') }}</button>
            <button class="button button-primary" @click="pendingArchive.action === 'restore' ? workspace.restoreArchive(pendingArchive.entry) : workspace.deleteArchive(pendingArchive.entry); pendingArchive = null">{{ pendingArchive.action === 'restore' ? t('Restore') : t('Remove') }}</button>
          </div>
        </div>
      </div>
      <footer class="profiler-footer"><span>SymfoLinker <span class="muted">· {{ t('Read-only scanner') }}</span></span><span>{{ t('Composer workspace tools') }}</span></footer>
    </div>
  </div>
</template>
