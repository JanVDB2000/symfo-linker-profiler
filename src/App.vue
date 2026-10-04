<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { isTauri } from '@tauri-apps/api/core'
import { useWorkspace } from './stores/workspace'
import ProfilerIcon from './components/ProfilerIcon.vue'
import type { PackageStatus } from './types'
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
const tabs = [
  { name: 'Projects', icon: 'projects' }, { name: 'Active links', icon: 'links' },
  { name: 'Runtime', icon: 'runtime' }, { name: 'Health', icon: 'health' },
  { name: 'Settings', icon: 'settings' },
]
const ARROW = '→'
const HOOK_ARROW = '↳'
const localCount = computed(() => workspace.selected?.packages.filter(p => p.mode === 'local').length ?? 0)
const issues = computed(() => workspace.selected?.packages.filter(p => p.mode === 'unknown' || (p.mode === 'local' && p.backupStatus !== 'available')) ?? [])
// Confirmation state for a pending switch; null while no dialog is open.
const pendingSwap = ref<{ pkg: PackageStatus; target: 'local' | 'vendor' } | null>(null)
function askSwap(pkg: PackageStatus, target: 'local' | 'vendor') {
  if (pkg.mode === target || workspace.swapping) return
  pendingSwap.value = { pkg, target }
}
async function confirmSwap() {
  const pending = pendingSwap.value
  if (!pending) return
  pendingSwap.value = null
  await workspace.setMode(pending.pkg, pending.target)
}

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
      <div class="header-search"><ProfilerIcon name="search" /><input v-model="workspace.search" type="search" :placeholder="t('Search projects')" :aria-label="t('Filter projects')" /></div>
    </header>
      <section class="request-summary" :class="{ 'summary-error': workspace.error }" :aria-label="t('Workspace status')" aria-live="polite">
        <div class="request-title"><span class="request-method">{{ t('SCAN') }}</span><h1>{{ workspace.error ? t('Scan incomplete') : workspace.result?.developmentRoot ?? statusTitle }}</h1></div>
        <div class="request-metadata"><span><b>{{ t('Status') }}</b> <span class="status-code">{{ workspace.error ? t('ERROR') : workspace.result ? 'OK' : t('LOCAL') }}</span> <span class="status-text">{{ workspace.error ? t('Scan incomplete') : statusTitle }}</span></span><span><b>{{ t('Project') }}</b> {{ workspace.selected?.name ?? '—' }}</span><span><b>{{ t('Runtime') }}</b> {{ workspace.selected ? workspace.selected.composeFile ? 'Docker Compose' : t('Native') : '—' }}</span><span><b>{{ t('Access') }}</b> {{ t('Read only') }}</span></div>
      </section>
      <div class="profiler-layout">
        <aside class="sidebar">
          <div class="sidebar-contents">
          <div class="sidebar-shortcuts"><button class="text-button" :aria-expanded="workspaceControls || !workspace.result" aria-controls="workspace-controls" @click="workspaceControls = !workspaceControls"><ProfilerIcon name="search" />{{ t('Workspace') }}</button><button class="text-button" :disabled="workspace.busy || !workspace.root" @click="workspace.scan()">{{ workspace.busy ? t('Scanning…') : t('Refresh') }}</button></div>
          <form v-if="workspaceControls || !workspace.result" id="workspace-controls" class="root-form" @submit.prevent="workspace.scan()"><label for="root">{{ t('Development root') }}</label><div class="root-controls"><input id="root" v-model="workspace.root" :placeholder="t('D:\\dev or /home/user/dev')" :disabled="workspace.busy" /><button type="button" class="button button-small" :disabled="workspace.busy" @click="chooseRoot"><ProfilerIcon name="folder" />{{ t('Browse') }}</button><button class="button button-small" :disabled="workspace.busy">{{ workspace.busy ? t('Scanning…') : t('Scan') }}</button></div></form>
          <nav class="collector-menu" :aria-label="t('Main navigation')">
            <button v-for="item in tabs" :key="item.name" :class="{ active: tab === item.name }" :aria-current="tab === item.name ? 'page' : undefined" @click="tab = item.name"><ProfilerIcon :name="item.icon" /><span>{{ t(item.name) }}</span><span v-if="count(item.name) !== null" class="menu-count" :class="{ 'count-warning': item.name === 'Health' && count(item.name) }">{{ count(item.name) }}</span></button>
          </nav>
          </div>
          <div class="project-picker">
            <h2>{{ t('Workspace projects') }}</h2>
            <div v-if="workspace.result?.projects.length" class="picker-search"><ProfilerIcon name="search" /><input v-model="workspace.search" type="search" :placeholder="t('Filter projects')" :aria-label="t('Filter workspace projects')" /></div>
            <div class="project-list">
              <button v-for="project in workspace.projects" :key="project.id" class="project-button" :class="{ selected: project.id === workspace.selectedId }" :aria-pressed="project.id === workspace.selectedId" @click="workspace.selectedId = project.id; tab = 'Projects'"><ProfilerIcon name="folder" /><span>{{ project.name }}</span><span v-if="project.git?.dirty" class="dirty-dot" :title="t('Changed files')"></span></button>
              <p v-if="!workspace.projects.length" class="sidebar-empty">{{ pickerEmpty }}</p>
            </div>
          </div>
          <div class="sidebar-settings"><button class="text-button" @click="tab = 'Settings'"><ProfilerIcon name="settings" />{{ t('Settings') }}</button><button class="theme-toggle" :aria-label="resolvedTheme === 'dark' ? t('Activate light theme') : t('Activate dark theme')" @click="theme = resolvedTheme === 'dark' ? 'light' : 'dark'"><ProfilerIcon :name="resolvedTheme === 'dark' ? 'sun' : 'moon'" /></button><button class="theme-toggle" :aria-pressed="wide" :aria-label="wide ? t('Normal width') : t('Full width')" @click="wide = !wide"><ProfilerIcon :name="wide ? 'collapse' : 'expand'" /></button></div>
        </aside>
        <main class="collector-content">
          <div class="collector-heading"><h2>{{ t(tab) }}</h2></div>
          <div v-if="workspace.error" class="notice error" role="alert">{{ translateMessage(workspace.error) }}</div>
          <details v-if="workspace.result?.warnings.length" class="notice warning" open><summary>{{ t('{count} scan issue(s)', { count: workspace.result.warnings.length }) }}</summary><ul><li v-for="warning in workspace.result.warnings" :key="JSON.stringify(warning)">{{ translateMessage(warning) }}</li></ul></details>
          <template v-if="tab === 'Projects'">
            <div class="metrics"><div class="metric"><strong>{{ workspace.result?.projects.length ?? '—' }}</strong><span>{{ t('Projects') }}</span></div><div class="metric"><strong>{{ workspace.selected?.packages.length ?? '—' }}</strong><span>{{ t('Local dependencies') }}</span></div><div class="metric"><strong>{{ workspace.selected ? localCount : '—' }}</strong><span>{{ t('Active links') }}</span></div><div class="metric"><strong :class="{ 'text-warning': issues.length }">{{ workspace.selected ? issues.length : '—' }}</strong><span>{{ t('Issues') }}</span></div></div>
            <template v-if="workspace.selected">
              <h3 class="heading-live">{{ t('Project details') }} <small>{{ workspace.selected.name }}</small><span class="heading-actions"><span v-if="checkedAt" class="live" :title="t('Sync every 5 minutes while this project is selected')"><span class="live-dot"></span>{{ t('updated {time}', { time: checkedAt }) }}</span><button class="button button-small" :disabled="workspace.statusBusy" :title="t('Refresh Git and container status for {project} now', { project: workspace.selected.name })" @click="workspace.refreshStatus()"><ProfilerIcon name="refresh" />{{ workspace.statusBusy ? t('Syncing...') : t('Sync') }}</button></span></h3>
              <div class="table-scroll"><table class="property-table"><tbody>
                <tr><th scope="row">{{ t('Composer package') }}</th><td>{{ workspace.selected.composerName ?? t('Not configured') }}</td></tr>
                <tr><th scope="row">{{ t('Project path') }}</th><td class="path">{{ workspace.selected.path }}</td></tr>
                <tr><th scope="row">{{ t('Git branch') }}</th><td><span class="branch"><ProfilerIcon name="branch" />{{ workspace.selected.git?.branch ?? t('No Git information') }}</span><code v-if="workspace.selected.git?.commit" class="commit">{{ workspace.selected.git.commit }}</code></td></tr>
                <tr><th scope="row">{{ t('Working directory') }}</th><td><span v-if="workspace.selected.git" class="label" :class="workspace.selected.git.dirty ? 'status-warning' : 'status-success'">{{ workspace.selected.git.dirty ? t('{count} changed files', { count: workspace.selected.git.changedFiles }) : t('Clean') }}</span><span v-else class="muted">{{ t('Unavailable') }}</span></td></tr>
                <tr><th scope="row">{{ t('Containers') }}</th><td><span class="label" :class="dockerLabel.cls">{{ dockerLabel.text }}</span><span v-if="docker?.message" class="muted docker-note">{{ translateMessage(docker.message) }}</span><span v-else-if="workspace.statusError" class="muted docker-note">{{ translateMessage(workspace.statusError) }}</span></td></tr>
                <tr><th scope="row">{{ t('Used by') }}</th><td><template v-if="usedBy.length"><button v-for="name in usedBy" :key="name" class="text-button chain-link" @click="workspace.selectedId = name; tab = 'Projects'">{{ name }}</button></template><span v-else class="muted">{{ t('No project links to this project') }}</span></td></tr>
              </tbody></table></div>
              <h3>{{ t('Local dependencies') }} <small>{{ workspace.selected.packages.length }}</small></h3>
              <div v-if="!workspace.selected.packages.length" class="empty"><p>{{ t('No dependencies with a unique local Composer package found.') }}</p></div>
              <div v-else class="table-scroll"><table class="packages-table"><thead><tr><th scope="col">{{ t('Package / constraint') }}</th><th scope="col">{{ t('Mode') }}</th><th scope="col">{{ t('Git branch') }}</th><th scope="col">{{ t('Host status') }}</th><th scope="col">{{ t('Backup') }}</th></tr></thead><tbody>
                <tr v-for="pkg in workspace.selected.packages" :key="`${pkg.packageName}-${pkg.dependencyType}`">
                  <td><strong class="package-name">{{ pkg.packageName }}</strong><div class="package-constraint">{{ pkg.constraint }} <span v-if="pkg.dependencyType === 'requireDev'" class="label">require-dev</span></div><details class="package-paths"><summary>{{ t('Paths') }}</summary><dl><dt>{{ t('Local') }}</dt><dd>{{ pkg.localPath }}</dd><dt>{{ t('Vendor') }}</dt><dd>{{ pkg.vendorPath }}</dd><dt>{{ t('Backup') }}</dt><dd>{{ pkg.backupPath }}</dd></dl></details></td>
                  <td><div v-if="pkg.mode === 'unknown'" class="mode-switch"><span class="label status-warning">{{ t('UNKNOWN') }}</span></div>
                    <div v-else class="mode-switch" role="group" :aria-label="t('Mode for {name}', { name: pkg.packageName })">
                      <button class="mode-option" :class="{ active: pkg.mode === 'vendor' }" :aria-pressed="pkg.mode === 'vendor'" :disabled="!!workspace.swapping" @click="askSwap(pkg, 'vendor')">{{ t('VENDOR') }}</button>
                      <button class="mode-option" :class="{ active: pkg.mode === 'local' }" :aria-pressed="pkg.mode === 'local'" :disabled="!!workspace.swapping" @click="askSwap(pkg, 'local')">{{ t('LOCAL') }}</button>
                      <span v-if="workspace.swapping === pkg.packageName" class="mode-busy">{{ t('Switching...') }}</span>
                    </div></td>
                  <td><span class="branch"><ProfilerIcon name="branch" />{{ pkg.git?.branch ?? '—' }}</span><div v-if="pkg.git?.dirty" class="text-warning">{{ t('{count} changed', { count: pkg.git.changedFiles }) }}</div></td>
                  <td :class="pkg.mode === 'unknown' ? 'text-warning' : 'text-success'">{{ status(pkg) }}</td>
                  <td :class="{ 'text-warning': pkg.mode === 'local' && pkg.backupStatus !== 'available' }">{{ t({ missing: 'Missing', available: 'Available', invalid: 'Invalid' }[pkg.backupStatus]) }}</td>
                </tr>
              </tbody></table></div>
              <div v-if="workspace.swapError" class="notice error" role="alert">{{ translateMessage(workspace.swapError) }}</div>
            </template>
            <div v-else class="empty empty-panel"><ProfilerIcon name="projects" /><h3>{{ workspace.result ? t('No Composer projects found') : t('Select your development root') }}</h3><p>{{ workspace.result ? t('The scanner looks for composer.json in immediate subdirectories. Choose the parent directory of your projects.') : t('Choose the directory containing your Composer projects. Their local dependencies, Git status and vendor paths will appear here.') }}</p><button class="button" :disabled="workspace.busy" @click="chooseRoot">{{ t('Choose development root') }}</button></div>
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
              <div v-if="!workspace.container" class="empty"><p>{{ t('Inspect the container to map host paths and validate linked packages.') }}</p></div>
              <template v-else>
                <div v-if="workspace.container.message" class="notice warning">{{ translateMessage(workspace.container.message) }}</div>
                <div class="table-scroll"><table class="property-table"><tbody>
                  <tr><th scope="row">{{ t('PHP service') }}</th><td>
                    <select v-if="workspace.container.services.length" class="service-select" :value="workspace.container.phpService ?? ''" @change="workspace.selectPhpService(($event.target as HTMLSelectElement).value)">
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
            <div v-else class="empty"><p>{{ t('Scan your workspace and select a project.') }}</p></div><p class="help">{{ t('Status comes from docker compose ps in the project directory and refreshes automatically. Mount inspection and PHP-FPM validation will follow in the Docker milestone.') }}</p>
          </template>
          <template v-else-if="tab === 'Health'">
            <h3>{{ t('Environment') }}</h3><div class="table-scroll"><table class="property-table"><tbody><tr><th scope="row">{{ t('Development root') }}</th><td><span class="label" :class="workspace.result ? 'status-success' : ''">{{ workspace.result ? t('Readable') : t('Not scanned yet') }}</span></td></tr><tr><th scope="row">{{ t('Composer projects') }}</th><td>{{ workspace.result?.projects.length ?? '—' }}</td></tr><tr><th scope="row">{{ t('Git information') }}</th><td>{{ workspace.selected?.git ? t('Available') : t('Unavailable') }}</td></tr><tr><th scope="row">{{ t('Package issues') }}</th><td>{{ workspace.selected ? issues.length : '—' }}</td></tr></tbody></table></div>
            <h3>{{ t('Packages') }} <small>{{ workspace.selected?.name }}</small></h3><div v-for="pkg in issues" :key="`${pkg.packageName}-${pkg.dependencyType}`" class="notice warning"><strong>{{ pkg.packageName }}</strong> — {{ pkg.mode === 'local' ? t('Local link has no valid vendor backup.') : status(pkg) }}</div><div v-if="!issues.length" class="empty"><p>{{ workspace.selected ? t('No package issues found.') : t('Select a scanned project to check its packages.') }}</p></div><p class="help">{{ t('This check reports readability and path status. Write permissions and containers are not checked yet.') }}</p>
          </template>
          <template v-else-if="tab === 'Settings'">
            <h3>{{ t('Appearance') }}</h3><div class="table-scroll"><table class="property-table"><tbody><tr><th scope="row"><label for="language">{{ t('Language') }}</label></th><td><select id="language" v-model="locale"><option v-for="language in languages" :key="language.code" :value="language.code">{{ language.label }}</option></select></td></tr><tr><th scope="row">{{ t('Theme') }}</th><td><div class="theme-options" role="group" :aria-label="t('Color theme')"><button class="button" :aria-pressed="theme === 'auto'" @click="theme = 'auto'">{{ t('Automatic') }}</button><button class="button" :aria-pressed="theme === 'light'" @click="theme = 'light'"><ProfilerIcon name="sun" />{{ t('Light') }}</button><button class="button" :aria-pressed="theme === 'dark'" @click="theme = 'dark'"><ProfilerIcon name="moon" />{{ t('Dark') }}</button></div></td></tr><tr><th scope="row">{{ t('Scan scope') }}</th><td>{{ t('Immediate subdirectories of the development root') }}</td></tr><tr><th scope="row">{{ t('Project access') }}</th><td><span class="label status-success">{{ t('Read only') }}</span></td></tr></tbody></table></div><h3>{{ t('Workspace') }}</h3><p>{{ t('Use Workspace in the sidebar to change the development root. The last scanned root, theme and language are saved locally.') }}</p><p class="help">{{ t('This version reads Composer files, Git status and vendor paths. The scanner does not modify project files.') }}</p>
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
            <p>{{ t('Local source:') }}</p>
            <p class="path dialog-path">{{ pendingSwap.pkg.localPath }}</p>
          </template>
          <p v-else>{{ t('The local link will be removed and the preserved Composer package restored.') }}</p>
          <div class="dialog-actions">
            <button class="button" @click="pendingSwap = null">{{ t('Cancel') }}</button>
            <button class="button button-primary" @click="confirmSwap">{{ pendingSwap.target === 'local' ? t('Use Local') : t('Restore Vendor') }}</button>
          </div>
        </div>
      </div>
      <footer class="profiler-footer"><span>SymfoLinker <span class="muted">· {{ t('Read-only scanner') }}</span></span><span>{{ t('Composer workspace tools') }}</span></footer>
    </div>
  </div>
</template>
