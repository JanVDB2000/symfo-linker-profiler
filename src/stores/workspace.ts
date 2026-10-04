import { computed, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { isMessage } from '../i18n'
import type { Message } from '../i18n'
import type { ContainerReport, GitInfo, LinkedPackage, PackageStatus, Project, ProjectStatus, ScanResult } from '../types'

// Add a short fixture delay so the busy state is visible during development.
const FIXTURE_DELAY_MS = 400

// Refresh the selected project sparingly: each poll starts Git and Docker processes.
// The Sync button allows an immediate refresh.
const POLL_INTERVAL_MS = 5 * 60 * 1000

export const useWorkspace = defineStore('workspace', () => {
  const root = ref(localStorage.getItem('symfolinker.root') ?? '')
  const result = ref<ScanResult | null>(null)
  const selectedId = ref('')
  const busy = ref(false)
  const error = ref<Message | string>('')
  const search = ref('')
  const status = ref<ProjectStatus | null>(null)
  const statusError = ref<Message | string>('')
  const lastChecked = ref(0)
  const statusBusy = ref(false)
  // Package currently being switched; empty when idle. Guards against a second click.
  const swapping = ref('')
  const swapError = ref<Message | string>('')
  // Container inspection is explicit, never polled: it runs compose config plus
  // one exec per linked package.
  const container = ref<ContainerReport | null>(null)
  const containerBusy = ref(false)
  const selected = computed(() => result.value?.projects.find(p => p.id === selectedId.value))
  const projects = computed(() => result.value?.projects.filter(p =>
    `${p.name} ${p.composerName ?? ''}`.toLowerCase().includes(search.value.toLowerCase())) ?? [])
  const links = computed(() => result.value?.projects.flatMap(project =>
    project.packages.filter(p => p.mode === 'local').map(pkg => ({ project, pkg }))) ?? [])
  // Group by local package rather than by project: the question the dashboard answers
  // is "where is this bundle used, and on which branch", which spans projects.
  const linkedPackages = computed(() => {
    const byPackage = links.value.reduce((map, { project, pkg }) => map.set(pkg.packageName, {
      packageName: pkg.packageName,
      localProjectId: pkg.localProjectId,
      localPath: pkg.localPath,
      // The local source's branch: what a consumer actually gets when linked.
      git: pkg.git,
      usedBy: [...(map.get(pkg.packageName)?.usedBy ?? []), { id: project.id, name: project.name }],
    }), new Map<string, LinkedPackage>())
    return [...byPackage.values()].sort((a, b) => a.packageName.localeCompare(b.packageName))
  })
  // Linked projects can have their own links, creating chains such as A -> D -> C.
  const linkedBy = computed(() => links.value.reduce((map, { project, pkg }) => {
    const ids = map.get(pkg.localProjectId) ?? []
    return ids.includes(project.id) ? map : map.set(pkg.localProjectId, [...ids, project.id])
  }, new Map<string, string[]>()))
  const linksFrom = computed(() => links.value.reduce((map, { project, pkg }) => {
    const names = map.get(project.id) ?? []
    return map.set(project.id, [...names, pkg.packageName])
  }, new Map<string, string[]>()))

  // Browser development uses fixtures because the Rust scanner is unavailable.
  async function loadFixture() {
    busy.value = true
    error.value = ''
    try {
      const { devFixture } = await import('../dev/fixture')
      await new Promise(resolve => setTimeout(resolve, FIXTURE_DELAY_MS))
      if (new URLSearchParams(location.search).has('error')) {
        result.value = null
        error.value = { key: 'Scanning failed: {path} is unreadable. (dev simulation via ?error)', params: { path: 'D:\\dev' } }
        return
      }
      result.value = devFixture
      root.value = devFixture.developmentRoot
      if (!devFixture.projects.some(p => p.id === selectedId.value)) selectedId.value = devFixture.projects[0]?.id ?? ''
    } finally { busy.value = false }
  }

  async function scan() {
    if (busy.value) return
    error.value = ''
    if (!isTauri()) {
      // Production builds must never present fixture data as real scans.
      if (import.meta.env.DEV) { await loadFixture(); return }
      error.value = { key: 'Open the desktop app with npm run desktop to scan local projects.' }
      return
    }
    if (!root.value.trim()) { error.value = { key: 'Choose a development root first.' }; return }
    busy.value = true
    try {
      const next = await invoke<ScanResult>('scan_projects', { developmentRoot: root.value.trim() })
      result.value = next
      root.value = next.developmentRoot
      localStorage.setItem('symfolinker.root', next.developmentRoot)
      if (!next.projects.some(p => p.id === selectedId.value)) selectedId.value = next.projects[0]?.id ?? ''
    } catch (cause) {
      result.value = null
      error.value = isMessage(cause) ? cause : { key: 'Scanning failed. Check the selected path and read permissions.' }
    } finally { busy.value = false }
  }
  // Replace project Git state immutably so the sidebar, metrics and table update together.
  // Preserve the other projects and scan metadata.
  function applyGit(id: string, git: GitInfo | null) {
    const current = result.value
    const project = current?.projects.find(p => p.id === id)
    if (!current || !project || JSON.stringify(project.git) === JSON.stringify(git)) return
    result.value = { ...current, projects: current.projects.map(p => p.id === id ? { ...p, git } : p) }
  }

  /// Switches a package and adopts the state the backend read back from disk.
  ///
  /// Plan section 48: no optimistic state for filesystem mutations. Nothing in the
  /// interface changes until the command returns a fresh scan.
  async function setMode(pkg: PackageStatus, target: 'local' | 'vendor') {
    const project = selected.value
    if (!project || swapping.value) return
    swapping.value = pkg.packageName
    swapError.value = ''
    try {
      const next = await requestSwap(project.id, pkg.packageName, target)
      if (!next) return
      result.value = next
      void refreshStatus()
    } catch (cause) {
      swapError.value = isMessage(cause) ? cause : { key: 'The switch could not be completed.' }
    } finally { swapping.value = '' }
  }

  // Same DEV guard as scan(): the fixture must stay out of production builds.
  async function requestSwap(projectId: string, packageName: string, target: 'local' | 'vendor') {
    if (isTauri()) {
      const command = target === 'local' ? 'activate_local' : 'activate_vendor'
      return await invoke<ScanResult>(command, { developmentRoot: root.value, projectId, packageName })
    }
    if (import.meta.env.DEV && result.value) {
      await new Promise(resolve => setTimeout(resolve, FIXTURE_DELAY_MS))
      return (await import('../dev/fixture')).devSwap(result.value, projectId, packageName, target)
    }
    throw { key: 'Open the desktop app with npm run desktop to switch packages.' }
  }

  async function inspectContainer() {
    const project = selected.value
    if (!project || containerBusy.value) return
    containerBusy.value = true
    try {
      container.value = isTauri()
        ? await invoke<ContainerReport>('inspect_container', { developmentRoot: root.value, projectId: project.id })
        : import.meta.env.DEV ? (await import('../dev/fixture')).devContainerReport(project) : null
    } catch (cause) {
      container.value = null
      swapError.value = isMessage(cause) ? cause : { key: 'The container could not be inspected.' }
    } finally { containerBusy.value = false }
  }

  async function selectPhpService(service: string) {
    const project = selected.value
    if (!project) return
    if (isTauri()) {
      await invoke('set_php_service', { developmentRoot: root.value, projectPath: project.path, service: service || null })
    }
    await inspectContainer()
  }

  let polling: ReturnType<typeof setInterval> | null = null

  // Keep the same DEV guard as scan() so Vite excludes fixtures from production.
  // Only browser development should load this module.
  async function readStatus(project: Project): Promise<ProjectStatus | null> {
    if (isTauri()) return await invoke<ProjectStatus>('project_status', { projectPath: project.path })
    if (import.meta.env.DEV) return (await import('../dev/fixture')).devProjectStatus(project)
    return null
  }

  async function refreshStatus() {
    const project = selected.value
    if (!project || statusBusy.value) return
    statusBusy.value = true
    const requestedId = project.id
    try {
      const next = await readStatus(project)
      // Ignore responses for a project that is no longer selected.
      if (!next || selected.value?.id !== requestedId) return
      status.value = next
      statusError.value = ''
      lastChecked.value = Date.now()
      applyGit(requestedId, next.git)
    } catch (cause) {
      if (selected.value?.id !== requestedId) return
      statusError.value = isMessage(cause) ? cause : { key: 'The project status could not be read.' }
    } finally { statusBusy.value = false }
  }

  function stopPolling() {
    if (polling === null) return
    clearInterval(polling)
    polling = null
  }

  function startPolling() {
    stopPolling()
    if (!selected.value || (!isTauri() && !import.meta.env.DEV)) return
    void refreshStatus()
    polling = setInterval(() => void refreshStatus(), POLL_INTERVAL_MS)
  }

  // Watch the ID instead of the object: applyGit replaces the project object.
  // This prevents the refresh from triggering its own watcher.
  watch(() => selected.value?.id, () => {
    status.value = null
    statusError.value = ''
    lastChecked.value = 0
    startPolling()
  })
  // Stop Git and Docker polling while the window is hidden.
  document.addEventListener('visibilitychange', () => document.hidden ? stopPolling() : startPolling())

  return { root, result, selectedId, busy, error, search, status, statusError, lastChecked, statusBusy, swapping, swapError, container, containerBusy, selected, projects, links, linkedPackages, linkedBy, linksFrom, scan, refreshStatus, startPolling, stopPolling, setMode, inspectContainer, selectPhpService }
})
