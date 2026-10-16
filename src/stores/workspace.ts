import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { isMessage } from '../i18n'
import type { Message } from '../i18n'
import type { ArchiveEntry, AutoloadDump, ContainerReport, ExternalTool, GitInfo, LinkedPackage, MountSync, PackageStatus, PreflightReport, Project, ProjectStatus, ScanResult } from '../types'
import { clearScan, readScan, readSelected, writeScan, writeSelected, readWorkspaces, writeWorkspaces, readProfiles, writeProfiles } from './scanCache'
import type { LinkProfile, ProfileEntry } from './scanCache'

// Add a short fixture delay so the busy state is visible during development.
const FIXTURE_DELAY_MS = 400

// Refresh the selected project sparingly: each poll starts Git and Docker processes.
// The Sync button allows an immediate refresh.
const POLL_INTERVAL_MS = 5 * 60 * 1000

// Linking two or three packages in a row is one move, not three. Waiting briefly for
// the next switch turns that into a single container recreation instead of one each,
// which is the difference between a pause and a workflow.
const MOUNT_SYNC_DELAY_MS = 900

// Keep the project that was open before, as long as the restored scan still has it.
function restoreSelection(result: ScanResult): string {
  const saved = readSelected()
  return result.projects.some(p => p.id === saved) ? saved : result.projects[0]?.id ?? ''
}

export const useWorkspace = defineStore('workspace', () => {
  const root = ref(localStorage.getItem('symfolinker.root') ?? '')
  const savedWorkspaces = ref(readWorkspaces())
  const profiles = ref(readProfiles())
  const workspaceProfiles = computed(() => profiles.value.filter(profile => profile.root === result.value?.developmentRoot))
  /// Why the current state cannot be saved as a profile, or '' when it can.
  ///
  /// A profile replays modes onto disk later, so it may only be taken from a scan this
  /// session verified, with every package in a state worth replaying. Keeping the
  /// reason next to the rule is what lets the button explain itself instead of just
  /// going grey.
  const profileBlocked = computed<Message | ''>(() => {
    const current = result.value
    if (!current || root.value !== current.developmentRoot) return { key: 'Scan this development root before saving a profile.' }
    if (busy.value || swapping.value) return { key: 'Wait until the workspace is idle.' }
    if (fromCache.value) return { key: 'Refresh first: a profile records what is really on disk, not what was restored from the last session.' }
    if (!current.projects.some(project => project.packages.length > 0)) return { key: 'There are no local dependencies to record.' }
    if (current.projects.some(project => project.packages.some(pkg => pkg.mode === 'unknown'))) return { key: 'Some packages are in an unknown state. Resolve them under Health before saving a profile.' }
    return ''
  })
  const canSaveProfile = computed(() => !profileBlocked.value)
  // Reopening the app restores the last scan, so no filesystem walk is needed to see
  // the workspace again. Refresh replaces it with a fresh read.
  const cached = readScan(root.value)
  const result = ref<ScanResult | null>(cached?.result ?? null)
  // When the data on screen was read from disk; 0 while nothing has been scanned.
  const scannedAt = ref(cached?.scannedAt ?? 0)
  // True while the interface shows restored data that this session has not verified.
  const fromCache = ref(cached !== null)
  const selectedId = ref(cached ? restoreSelection(cached.result) : '')
  const busy = ref(false)
  const error = ref<Message | string>('')
  const search = ref('')
  const status = ref<ProjectStatus | null>(null)
  const statusError = ref<Message | string>('')
  const lastChecked = ref(0)
  const statusBusy = ref(false)
  // Package currently being switched; empty when idle. Guards against a second click.
  const swapping = ref('')
  const mutationBlocked = ref(false)
  const swapError = ref<Message | string>('')
  // Container inspection is explicit, never polled: it runs compose config plus
  // one exec per linked package.
  const container = ref<ContainerReport | null>(null)
  const containerBusy = ref(false)
  const containerError = ref<Message | string>('')
  // Mounts follow the links on disk: a package switched to LOCAL needs its source
  // visible inside the container, one switched back to VENDOR must lose that mount
  // again. This runs after every switch, with its own busy state rather than reusing
  // swapping, because recreating a container takes far longer than the swap itself.
  const mountSync = ref<MountSync | null>(null)
  const mountSyncBusy = ref('')
  const mountSyncError = ref<Message | string>('')
  // What would put the last switch back, and what to call it on the button. Null when
  // there is nothing to undo: nothing has been switched, the last one failed, or the
  // undo itself has been applied. Deliberately not persisted, because it describes a
  // state of disk this session verified.
  const lastSwitch = ref<{ label: string; entries: ProfileEntry[] } | null>(null)
  // The preserved versions of the selected project, read on request: listing them
  // walks the history directory and measures every archive.
  const archives = ref<ArchiveEntry[]>([])
  const archivesBusy = ref(false)
  const archivesError = ref<Message | string>('')
  // Write checks are explicit for a stronger reason: they write. Everything that
  // happens by itself in this app reads, and that promise has to keep holding.
  const preflight = ref<PreflightReport | null>(null)
  const preflightBusy = ref(false)
  const preflightError = ref<Message | string>('')
  let statusRequest = 0
  let containerRequest = 0
  let archiveRequest = 0
  let preflightRequest = 0
  let contextVersion = 0
  const selected = computed(() => result.value?.projects.find(p => p.id === selectedId.value))
  // Searching a bundle name finds the applications that consume it, not just a project
  // called that: you usually know which package you are about to work on, and have to
  // reach it through whichever app happens to require it.
  const projects = computed(() => {
    const query = search.value.toLowerCase()
    return result.value?.projects.filter(p =>
      // The id rather than the name, so the group a project sits in is searchable
      // too: in a grouped root, "library" asks for everything that team maintains.
      `${p.id} ${p.composerName ?? ''}`.toLowerCase().includes(query)
      || p.packages.some(pkg => pkg.packageName.toLowerCase().includes(query))) ?? []
  })
  // The picker lists projects under the directory they live in, so a root grouped by
  // team or client reads the way it does on disk. A flat root yields a single group
  // with no heading, which is the list it has always been. Grouping by key rather
  // than by adjacency, so the order projects arrive in cannot split a group in two.
  const projectGroups = computed(() => [...projects.value.reduce((groups, project) => {
    const group = project.group ?? ''
    return groups.set(group, [...(groups.get(group) ?? []), project])
  }, new Map<string, Project[]>())].map(([group, items]) => ({ group, projects: items })))
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
  // Packages whose link something outside this app replaced: the journal recorded a
  // finished link and a Composer directory occupies the vendor path now. Across the
  // whole workspace, because one `composer install` run rarely stops at one project.
  const relinkablePackages = computed(() => result.value?.projects.flatMap(project =>
    project.packages.filter(pkg => pkg.relinkable).map(pkg => ({
      projectId: project.id, projectName: project.name, packageName: pkg.packageName,
    }))) ?? [])
  /// Whether a filesystem mutation may start now. One answer for every caller: a
  /// second one would drift from this one the first time a reason is added.
  const mutationReady = () => !busy.value && !swapping.value && !mountSyncBusy.value
    && !mutationBlocked.value && !!result.value && root.value === result.value.developmentRoot

  // Browser development uses fixtures because the Rust scanner is unavailable.
  async function loadFixture() {
    busy.value = true
    error.value = ''
    try {
      const { devFixture } = await import('../dev/fixture')
      await new Promise(resolve => setTimeout(resolve, FIXTURE_DELAY_MS))
      if (new URLSearchParams(location.search).has('error')) {
        forgetScan()
        error.value = { key: 'Scanning failed: {path} is unreadable. (dev simulation via ?error)', params: { path: 'D:\\dev' } }
        return
      }
      scannedAt.value = Date.now()
      fromCache.value = false
      mutationBlocked.value = false
      result.value = devFixture
      root.value = devFixture.developmentRoot
      if (!devFixture.projects.some(p => p.id === selectedId.value)) selectedId.value = devFixture.projects[0]?.id ?? ''
    } finally { busy.value = false }
  }

  async function scan() {
    if (busy.value || swapping.value) return
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
      scannedAt.value = Date.now()
      fromCache.value = false
      mutationBlocked.value = false
      result.value = next
      root.value = next.developmentRoot
      resetLiveState()
      startPolling()
      if (!savedWorkspaces.value.some(item => item.root === next.developmentRoot)) {
        savedWorkspaces.value.push({ root: next.developmentRoot, name: next.developmentRoot })
        writeWorkspaces(savedWorkspaces.value)
      }
      localStorage.setItem('symfolinker.root', next.developmentRoot)
      if (!next.projects.some(p => p.id === selectedId.value)) selectedId.value = next.projects[0]?.id ?? ''
      swapError.value = ''
    } catch (cause) {
      if (result.value?.developmentRoot === root.value.trim()) {
        fromCache.value = true
        mutationBlocked.value = true
        resetLiveState()
        stopPolling()
        clearScan()
      } else { forgetScan() }
      error.value = isMessage(cause) ? cause : { key: 'Scanning failed. Check the selected path and read permissions.' }
    } finally { busy.value = false }
  }

  /// Drops the stored scan together with the data on screen: the cache never outlives
  /// what the interface shows, so a restart cannot resurrect a discarded workspace.
  function forgetScan() {
    result.value = null
    scannedAt.value = 0
    fromCache.value = false
    selectedId.value = ''
    // There is no workspace left for an undo to describe.
    lastSwitch.value = null
  }

  async function openWorkspace(path: string) {
    if (busy.value || swapping.value) return
    forgetScan()
    root.value = path
    search.value = ''
    await scan()
  }

  function removeWorkspace(path: string) {
    savedWorkspaces.value = savedWorkspaces.value.filter(item => item.root !== path)
    writeWorkspaces(savedWorkspaces.value)
  }

  function saveProfile(name: string) {
    const current = result.value
    if (!current || !name.trim() || !canSaveProfile.value) return false
    const candidates = current.projects.flatMap(project => project.packages.map(pkg => ({
      projectId: project.id, packageName: pkg.packageName, mode: pkg.mode,
    })))
    const entries = [...new Map(candidates.map(entry => [`${entry.projectId}\0${entry.packageName}`, entry])).values()]
    if (!entries.length || entries.some(entry => entry.mode === 'unknown')) return false
    const profile = { name: name.trim(), root: current.developmentRoot, entries } as LinkProfile
    profiles.value = [...profiles.value.filter(item => item.root !== profile.root || item.name !== profile.name), profile]
    writeProfiles(profiles.value)
    return true
  }

  function removeProfile(name: string) {
    profiles.value = profiles.value.filter(item => item.root !== result.value?.developmentRoot || item.name !== name)
    writeProfiles(profiles.value)
  }

  /// Adopts the state the backend read back from disk after a mutation.
  ///
  /// Plan section 48: no optimistic state for filesystem changes. Every mutating
  /// command answers with a fresh scan, and this is what puts that scan on screen -
  /// in one place, so a new mutation cannot forget one of these five lines.
  function adoptScan(next: ScanResult) {
    scannedAt.value = Date.now()
    fromCache.value = false
    mutationBlocked.value = false
    result.value = next
    resetLiveState()
  }

  /// Runs one mutating command and adopts the scan it returns.
  ///
  /// The mutations added after the toggle differ only in their command, their label,
  /// which containers have to follow and what would undo them, so the bookkeeping
  /// lives here rather than in each of them.
  async function runMutation(
    label: string,
    command: string,
    args: Record<string, unknown>,
    follow: { projects: string[]; undo: ProfileEntry[] | null },
  ) {
    const current = result.value
    if (!current || !mutationReady()) return false
    swapping.value = label
    swapError.value = ''
    let switched = false
    try {
      if (!isTauri()) throw { key: 'Open the desktop app with npm run desktop to switch packages.' }
      const next = await invoke<ScanResult>(command, { developmentRoot: current.developmentRoot, ...args })
      adoptScan(next)
      void refreshStatus()
      lastSwitch.value = follow.undo?.length ? { label, entries: follow.undo } : null
      switched = true
    } catch (cause) {
      swapError.value = isMessage(cause) ? cause : { key: 'The switch could not be completed.' }
      await refreshAfterSwapError()
    } finally { swapping.value = '' }
    if (switched && follow.projects.length) scheduleMountSync(follow.projects)
    return switched
  }

  /// Switches every package back to LOCAL whose link an external install replaced.
  async function relinkPackages() {
    const pending = relinkablePackages.value
    if (!pending.length) return
    await runMutation('relink', 'relink_packages', { projectId: null }, {
      projects: [...new Set(pending.map(entry => entry.projectId))],
      // They were all on VENDOR a moment ago, which is what undoing this means.
      undo: pending.map(entry => ({ projectId: entry.projectId, packageName: entry.packageName, mode: 'vendor' as const })),
    })
  }

  /// Puts back what the last switch moved.
  ///
  /// It runs as a profile, so it is validated and reversed on failure like any other
  /// switch: an undo is not a shortcut past the checks, it is a switch in reverse.
  async function undoSwitch() {
    const previous = lastSwitch.value
    if (!previous) return
    await runMutation('undo', 'apply_profile', { entries: previous.entries }, {
      projects: [...new Set(previous.entries.map(entry => entry.projectId))], undo: null,
    })
  }

  async function refreshAfterSwapError() {
    const developmentRoot = result.value?.developmentRoot
    // A switch that failed may have been rolled back halfway; what the recorded undo
    // describes is no longer certain, so it is dropped rather than offered.
    lastSwitch.value = null
    if (!isTauri() || !developmentRoot) return
    try {
      const next = await invoke<ScanResult>('scan_projects', { developmentRoot })
      if (root.value !== developmentRoot) return
      adoptScan(next)
      if (!next.projects.some(project => project.id === selectedId.value)) selectedId.value = next.projects[0]?.id ?? ''
      void refreshStatus()
    } catch {
      // Keep the workspace visible if refreshing fails; label it stale and require
      // a successful refresh before further filesystem mutations.
      fromCache.value = true
      mutationBlocked.value = true
      resetLiveState()
      stopPolling()
      clearScan()
    }
  }

  async function applyProfile(profile: LinkProfile) {
    if (!mutationReady() || profile.root !== result.value?.developmentRoot || root.value !== profile.root) return
    // Only the packages this profile actually moves need putting back afterwards.
    const undo = profile.entries.flatMap(entry => {
      const pkg = result.value?.projects.find(project => project.id === entry.projectId)
        ?.packages.find(candidate => candidate.packageName === entry.packageName)
      return pkg && pkg.mode !== 'unknown' && pkg.mode !== entry.mode
        ? [{ projectId: entry.projectId, packageName: entry.packageName, mode: pkg.mode }]
        : []
    })
    swapping.value = profile.name
    swapError.value = ''
    let switched = false
    try {
      if (!isTauri()) throw { key: 'Open the desktop app with npm run desktop to switch packages.' }
      const next = await invoke<ScanResult>('apply_profile', { developmentRoot: profile.root, entries: profile.entries })
      adoptScan(next)
      void refreshStatus()
      lastSwitch.value = undo.length ? { label: profile.name, entries: undo } : null
      switched = true
    } catch (cause) {
      swapError.value = isMessage(cause) ? cause : { key: 'The switch could not be completed.' }
      await refreshAfterSwapError()
    } finally { swapping.value = '' }
    // A profile can switch packages in several projects; every container follows.
    if (switched) scheduleMountSync(profile.entries.map(entry => entry.projectId))
  }

  async function recoverProject() {
    const project = selected.value
    if (!project || !mutationReady()) return
    swapping.value = 'recovery'
    swapError.value = ''
    let switched = false
    try {
      if (!isTauri()) throw { key: 'Open the desktop app with npm run desktop to switch packages.' }
      const next = await invoke<ScanResult>('recover_project', { developmentRoot: result.value!.developmentRoot, projectId: project.id })
      adoptScan(next)
      void refreshStatus()
      // A repair is not a switch, and it may well have resolved the package the
      // recorded undo describes, so there is nothing sensible left to undo.
      lastSwitch.value = null
      switched = true
    } catch (cause) {
      swapError.value = isMessage(cause) ? cause : { key: 'The switch could not be completed.' }
      await refreshAfterSwapError()
    } finally { swapping.value = '' }
    // Recovery can roll a package back to VENDOR, which leaves a mount behind.
    if (switched) scheduleMountSync([project.id])
  }

  // Mirror every replacement of the scan, so what is on screen is what a restart restores.
  watch(result, current => current && !mutationBlocked.value ? writeScan(current, scannedAt.value) : clearScan())
  watch(selectedId, id => writeSelected(id))

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
    if (!project || !mutationReady()) return
    // Read before the switch, because that is what undoing it has to restore.
    const previousMode = pkg.mode
    swapping.value = pkg.packageName
    swapError.value = ''
    let switched = false
    try {
      const next = await requestSwap(project.id, pkg.packageName, target)
      if (!next) return
      adoptScan(next)
      void refreshStatus()
      if (previousMode !== 'unknown') {
        lastSwitch.value = { label: pkg.packageName, entries: [{ projectId: project.id, packageName: pkg.packageName, mode: previousMode }] }
      }
      switched = true
    } catch (cause) {
      swapError.value = isMessage(cause) ? cause : { key: 'The switch could not be completed.' }
      await refreshAfterSwapError()
    } finally { swapping.value = '' }
    // The link on disk changed, so the container's mounts have to follow it.
    if (switched) scheduleMountSync([project.id])
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

  // Runs synchronously up to its first request on purpose: the request counter below
  // is what keeps a late response from an earlier project off the screen, and an await
  // before it would let the selection change first.
  async function inspectContainer() {
    const project = selected.value
    if (!project || containerBusy.value) return
    containerBusy.value = true
    containerError.value = ''
    const request = ++containerRequest
    const context = contextVersion
    try {
      const next = isTauri()
        ? await invoke<ContainerReport>('inspect_container', { developmentRoot: root.value, projectId: project.id })
        : import.meta.env.DEV ? (await import('../dev/fixture')).devContainerReport(project) : null
      if (request === containerRequest && context === contextVersion) container.value = next
    } catch (cause) {
      if (request !== containerRequest || context !== contextVersion) return
      container.value = null
      containerError.value = isMessage(cause) ? cause : { key: 'The container could not be inspected.' }
    } finally { if (request === containerRequest) containerBusy.value = false }
  }

  async function selectPhpService(service: string) {
    const project = selected.value
    if (!project || containerBusy.value) return
    const context = contextVersion
    containerBusy.value = true
    containerError.value = ''
    try {
      if (isTauri()) await invoke('set_php_service', { developmentRoot: root.value, projectPath: project.path, service: service || null })
      if (context !== contextVersion) return
      containerBusy.value = false
      await inspectContainer()
    } catch (cause) {
      if (context === contextVersion) containerError.value = isMessage(cause) ? cause : { key: 'The PHP service could not be saved.' }
    } finally { if (context === contextVersion) containerBusy.value = false }
  }

  async function applyContainerMounts() {
    // The automatic pass may already cover this; let it run before adding to it.
    await flushMountSync()
    const project = selected.value
    const service = container.value?.phpService
    if (!project || !service || containerBusy.value || swapping.value || mountSyncBusy.value || busy.value || mutationBlocked.value || root.value !== result.value?.developmentRoot) return
    const context = contextVersion
    const request = ++containerRequest
    containerBusy.value = true
    containerError.value = ''
    try {
      if (!isTauri()) throw { key: 'Open the desktop app with npm run desktop to switch packages.' }
      const report = await invoke<ContainerReport>('apply_container_mounts', { developmentRoot: result.value!.developmentRoot, projectId: project.id, service })
      if (context !== contextVersion || request !== containerRequest) return
      container.value = report
      void refreshStatus()
    } catch (cause) {
      if (context !== contextVersion || request !== containerRequest) return
      containerError.value = isMessage(cause) ? cause : { key: 'The container could not be inspected.' }
    } finally { if (request === containerRequest) containerBusy.value = false }
  }

  // Projects waiting for a synchronisation, and the timer that will run them together.
  const pending = new Set<string>()
  let mountSyncTimer: ReturnType<typeof setTimeout> | null = null

  /// Queues projects for synchronisation and restarts the wait.
  ///
  /// Switching a second package within the delay joins the same pass, so a burst of
  /// switches recreates the container once rather than once per package.
  function scheduleMountSync(projectIds: string[]) {
    for (const projectId of projectIds) pending.add(projectId)
    if (mountSyncTimer) clearTimeout(mountSyncTimer)
    mountSyncTimer = setTimeout(() => void flushMountSync(), MOUNT_SYNC_DELAY_MS)
  }

  /// Runs the queued synchronisation now, for anything that cannot wait for the timer:
  /// inspecting the container, applying mounts by hand, or a test.
  async function flushMountSync() {
    if (mountSyncTimer) { clearTimeout(mountSyncTimer); mountSyncTimer = null }
    if (!pending.size) return
    // A pass is already running. It holds the development root's lock, so the queue
    // has to wait for it rather than fail against it.
    if (mountSyncBusy.value) { scheduleMountSync([]); return }
    const projectIds = [...pending]
    pending.clear()
    await syncContainerMounts(projectIds)
  }

  // A profile touches several projects, so their changes are reported as one result.
  const mergeSync = (current: MountSync | null, next: MountSync): MountSync => ({
    added: [...(current?.added ?? []), ...next.added],
    removed: [...(current?.removed ?? []), ...next.removed],
    recreated: (current?.recreated ?? false) || next.recreated,
  })

  /// Brings each project's container mounts in line with the links on disk.
  ///
  /// Called after every switch rather than from a button: the mount a LOCAL package
  /// needs, and the removal a VENDOR package implies, are part of the switch itself.
  /// The backend recreates the PHP service only when the mount set actually changed,
  /// so switching packages the container already sees restarts nothing.
  async function syncContainerMounts(projectIds: string[]) {
    const developmentRoot = result.value?.developmentRoot
    if (!isTauri() || !developmentRoot || root.value !== developmentRoot) return
    mountSync.value = null
    mountSyncError.value = ''
    for (const projectId of projectIds) {
      const context = contextVersion
      const request = ++containerRequest
      mountSyncBusy.value = projectId
      containerBusy.value = true
      try {
        const report = await invoke<ContainerReport>('sync_container_mounts', { developmentRoot, projectId })
        if (context !== contextVersion || request !== containerRequest) return
        // Only the project still on screen may replace the Runtime panel's report.
        if (selected.value?.id === projectId) container.value = report
        if (report.synced) mountSync.value = mergeSync(mountSync.value, report.synced)
      } catch (cause) {
        if (context !== contextVersion || request !== containerRequest) return
        mountSyncError.value = isMessage(cause) ? cause : { key: 'The container mounts could not be synchronised.' }
      } finally {
        if (request === containerRequest) { containerBusy.value = false; mountSyncBusy.value = '' }
      }
    }
    // A recreated service reports a new container id and uptime.
    if (mountSync.value?.recreated) void refreshStatus()
  }

  /// Reads the preserved versions of the selected project.
  ///
  /// Explicit, like inspecting a container: it walks the history directory and
  /// measures every archive, which is work nobody wants on every scan.
  async function loadArchives() {
    const project = selected.value
    const current = result.value
    if (!project || !current || archivesBusy.value) return
    archivesBusy.value = true
    archivesError.value = ''
    const request = ++archiveRequest
    const context = contextVersion
    try {
      const next = isTauri()
        ? await invoke<ArchiveEntry[]>('project_history', { developmentRoot: current.developmentRoot, projectId: project.id })
        : import.meta.env.DEV ? (await import('../dev/fixture')).devArchives(project) : []
      if (request !== archiveRequest || context !== contextVersion) return
      archives.value = next
    } catch (cause) {
      if (request !== archiveRequest || context !== contextVersion) return
      archives.value = []
      archivesError.value = isMessage(cause) ? cause : { key: 'The preserved versions could not be read.' }
    } finally { if (request === archiveRequest) archivesBusy.value = false }
  }

  /// Removes one preserved version for good.
  ///
  /// The only deletion in this app, which is why it changes nothing else: the active
  /// backup and the link stay exactly where they are, and the backend answers with
  /// what is left rather than this guessing.
  async function deleteArchive(entry: ArchiveEntry) {
    const project = selected.value
    const current = result.value
    if (!project || !current || archivesBusy.value || !mutationReady()) return
    archivesBusy.value = true
    archivesError.value = ''
    const request = ++archiveRequest
    const context = contextVersion
    try {
      if (!isTauri()) throw { key: 'Open the desktop app with npm run desktop to switch packages.' }
      const left = await invoke<ArchiveEntry[]>('delete_archive', {
        developmentRoot: current.developmentRoot, projectId: project.id,
        packageName: entry.packageName, archiveId: entry.archiveId,
      })
      if (request !== archiveRequest || context !== contextVersion) return
      archives.value = left
    } catch (cause) {
      if (request !== archiveRequest || context !== contextVersion) return
      archivesError.value = isMessage(cause) ? cause : { key: 'The preserved version could not be removed.' }
    } finally { if (request === archiveRequest) archivesBusy.value = false }
  }

  /// Makes a preserved version the one a switch to VENDOR will restore.
  ///
  /// No container follows this: the link does not move, only what is kept behind it,
  /// so the mounts are already right.
  async function restoreArchive(entry: ArchiveEntry) {
    const project = selected.value
    if (!project) return
    const done = await runMutation('archive', 'restore_archive', {
      projectId: project.id, packageName: entry.packageName, archiveId: entry.archiveId,
    }, { projects: [], undo: null })
    if (done) await loadArchives()
  }

  /// Writes a probe wherever a switch needs one, then removes it again.
  async function runPreflight() {
    const project = selected.value
    const current = result.value
    if (!project || !current || preflightBusy.value || !mutationReady()) return
    preflightBusy.value = true
    preflightError.value = ''
    const request = ++preflightRequest
    const context = contextVersion
    try {
      const next = isTauri()
        ? await invoke<PreflightReport>('preflight_checks', { developmentRoot: current.developmentRoot, projectId: project.id })
        : import.meta.env.DEV ? (await import('../dev/fixture')).devPreflight() : null
      if (request !== preflightRequest || context !== contextVersion) return
      preflight.value = next
    } catch (cause) {
      if (request !== preflightRequest || context !== contextVersion) return
      preflight.value = null
      preflightError.value = isMessage(cause) ? cause : { key: 'The write checks could not be completed.' }
    } finally { if (request === preflightRequest) preflightBusy.value = false }
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
    const request = ++statusRequest
    const context = contextVersion
    const requestedId = project.id
    try {
      const next = await readStatus(project)
      // Ignore responses for a project that is no longer selected.
      if (!next || request !== statusRequest || context !== contextVersion) return
      status.value = next
      statusError.value = ''
      lastChecked.value = Date.now()
      applyGit(requestedId, next.git)
    } catch (cause) {
      if (request !== statusRequest || context !== contextVersion) return
      statusError.value = isMessage(cause) ? cause : { key: 'The project status could not be read.' }
    } finally { if (request === statusRequest) statusBusy.value = false }
  }

  function stopPolling() {
    if (polling === null) return
    clearInterval(polling)
    polling = null
  }

  function startPolling() {
    stopPolling()
    if (document.hidden || !selected.value || (!isTauri() && !import.meta.env.DEV)) return
    void refreshStatus()
    polling = setInterval(() => void refreshStatus(), POLL_INTERVAL_MS)
    // A watcher reports a branch change or an install the moment it happens, which is
    // what the interval was approximating. It takes the interval's place as soon as
    // it is established, and the interval stays when it cannot be.
    void startWatching().then(() => {
      if (watching.value) stopPolling()
    })
  }

  // Watch the ID instead of the object: applyGit replaces the project object.
  // This prevents the refresh from triggering its own watcher.
  function resetLiveState() {
    contextVersion++
    statusRequest++
    containerRequest++
    archiveRequest++
    preflightRequest++
    statusBusy.value = false
    containerBusy.value = false
    container.value = null
    containerError.value = ''
    mountSync.value = null
    mountSyncError.value = ''
    status.value = null
    statusError.value = ''
    lastChecked.value = 0
    // Both describe a moment on disk that a switch or another project invalidates.
    archives.value = []
    archivesError.value = ''
    archivesBusy.value = false
    preflight.value = null
    preflightError.value = ''
    preflightBusy.value = false
  }
  // The autoload comparison reports a mismatch; this is the repair for it. Composer
  // runs in the project's PHP container when one is up, and on the host otherwise,
  // which is a difference worth showing rather than hiding.
  const autoloadDump = ref<AutoloadDump | null>(null)
  const autoloadBusy = ref(false)
  const autoloadError = ref<Message | string>('')

  /// Regenerates the selected project's autoloader, then reads the workspace again.
  ///
  /// The rescan is as much the point as the dump: "Autoload differs" is read from
  /// disk, so looking again is the only honest way to clear it.
  async function dumpAutoload() {
    const project = selected.value
    const current = result.value
    if (!project || !current || autoloadBusy.value || busy.value || !!swapping.value) return
    autoloadBusy.value = true
    autoloadError.value = ''
    autoloadDump.value = null
    const context = contextVersion
    try {
      const report = isTauri()
        ? await invoke<AutoloadDump>('dump_autoload', { developmentRoot: current.developmentRoot, projectId: project.id })
        : import.meta.env.DEV ? (await import('../dev/fixture')).devAutoloadDump() : null
      if (context !== contextVersion) return
      autoloadDump.value = report
      if (isTauri()) await scan()
    } catch (cause) {
      if (context !== contextVersion) return
      autoloadError.value = isMessage(cause) ? cause : { key: 'The autoloader could not be regenerated.' }
    } finally {
      autoloadBusy.value = false
    }
  }

  // Which tools this computer can be handed a path: only those are offered, so a
  // button never promises an editor that is not installed.
  const externalTools = ref<ExternalTool[]>([])
  const externalError = ref<Message | string>('')
  /// The path the last copy put on the clipboard, so the interface can confirm it.
  const copiedPath = ref('')

  async function loadExternalTools() {
    if (!isTauri()) {
      externalTools.value = import.meta.env.DEV ? ['phpstorm', 'vscode', 'terminal', 'files'] : []
      return
    }
    // A failure here means no buttons rather than an error: this is a convenience,
    // and it must never be the reason a workspace looks broken.
    try { externalTools.value = await invoke<ExternalTool[]>('external_tools') } catch { externalTools.value = [] }
  }

  async function openExternal(tool: ExternalTool) {
    const project = selected.value
    const current = result.value
    if (!project || !current) return
    externalError.value = ''
    if (!isTauri()) {
      externalError.value = { key: 'Open the desktop app with npm run desktop to scan local projects.' }
      return
    }
    try {
      await invoke('open_external', { developmentRoot: current.developmentRoot, projectId: project.id, tool })
    } catch (cause) {
      externalError.value = isMessage(cause) ? cause : { key: 'This tool could not be started.' }
    }
  }

  /// Copies a path to the clipboard without a plugin: the webview's own clipboard,
  /// with the older selection trick for the contexts that refuse it.
  async function copyPath(path: string) {
    externalError.value = ''
    copiedPath.value = ''
    try {
      await navigator.clipboard.writeText(path)
      copiedPath.value = path
      return
    } catch { /* The fallback below is what the refusal leaves. */ }
    try {
      const field = document.createElement('textarea')
      field.value = path
      field.setAttribute('readonly', '')
      field.style.position = 'fixed'
      field.style.opacity = '0'
      document.body.appendChild(field)
      field.select()
      const copied = document.execCommand('copy')
      document.body.removeChild(field)
      if (copied) copiedPath.value = path
      else externalError.value = { key: 'The path could not be copied to the clipboard.' }
    } catch {
      externalError.value = { key: 'The path could not be copied to the clipboard.' }
    }
  }

  // True while the backend watches the selected project. The five-minute poll is the
  // fallback for when it cannot: a network drive, a platform limit, a project that
  // disappeared between the scan and the watch.
  const watching = ref(false)
  let unlistenChanges: (() => void) | null = null

  /// Asks the backend to watch the selected project, and starts listening once.
  ///
  /// The listener is deliberately created lazily and only in the desktop app: in the
  /// browser there is no event bridge to import, and in the tests there is no Tauri.
  async function startWatching() {
    const project = selected.value
    const current = result.value
    watching.value = false
    if (!isTauri() || !project || !current || root.value !== current.developmentRoot) return
    try {
      if (!unlistenChanges) {
        const { listen } = await import('@tauri-apps/api/event')
        unlistenChanges = await listen<{ projectId: string; change: 'branch' | 'vendor' }>('project-changed', event => {
          // A selection that moved on in the meantime has a watch of its own.
          if (event.payload.projectId !== selectedId.value) return
          // An install or a replaced link changes what the scan reports; a branch
          // change only changes the status, which is far cheaper to read.
          if (event.payload.change === 'vendor') void scan()
          else void refreshStatus()
        })
      }
      await invoke('watch_project', { developmentRoot: current.developmentRoot, projectId: project.id })
      watching.value = true
    } catch {
      watching.value = false
    }
  }

  async function stopWatching() {
    watching.value = false
    if (!isTauri()) return
    try { await invoke('stop_watching') } catch { /* Nothing left to stop. */ }
  }

  watch(() => `${result.value?.developmentRoot ?? ''}\0${selected.value?.path ?? ''}`, () => {
    resetLiveState()
    startPolling()
  }, { flush: 'sync', immediate: true })
  // Stop Git and Docker polling while the window is hidden, and stop watching with
  // it: a refresh nobody can see is the cost this was meant to remove.
  const visibilityChanged = () => {
    if (!document.hidden) { startPolling(); return }
    stopPolling()
    void stopWatching()
  }
  document.addEventListener('visibilitychange', visibilityChanged)
  onScopeDispose(() => {
    stopPolling()
    void stopWatching()
    unlistenChanges?.()
    unlistenChanges = null
    if (mountSyncTimer) clearTimeout(mountSyncTimer)
    resetLiveState()
    document.removeEventListener('visibilitychange', visibilityChanged)
  })

  return { applyContainerMounts, syncContainerMounts, flushMountSync, mountSync, mountSyncBusy, mountSyncError, mutationBlocked, canSaveProfile, profileBlocked, recoverProject, savedWorkspaces, workspaceProfiles, openWorkspace, removeWorkspace, saveProfile, removeProfile, applyProfile, root, result, scannedAt, fromCache, selectedId, busy, error, search, status, statusError, lastChecked, statusBusy, swapping, swapError, container, containerBusy, containerError, selected, projects, projectGroups, links, linkedPackages, linkedBy, linksFrom, scan, refreshStatus, startPolling, stopPolling, setMode, inspectContainer, selectPhpService, forgetScan, relinkablePackages, relinkPackages, lastSwitch, undoSwitch, archives, archivesBusy, archivesError, loadArchives, deleteArchive, restoreArchive, preflight, preflightBusy, preflightError, runPreflight, autoloadDump, autoloadBusy, autoloadError, dumpAutoload, externalTools, externalError, copiedPath, loadExternalTools, openExternal, copyPath, watching, startWatching, stopWatching }
})
