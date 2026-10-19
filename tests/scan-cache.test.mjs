import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import test from 'node:test'
import { nextTick } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import ts from 'typescript'

const read = path => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8')
const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const transpile = source => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
const vueUrl = pathToFileURL(`${process.cwd()}/node_modules/vue/index.mjs`).href
const piniaUrl = pathToFileURL(`${process.cwd()}/node_modules/pinia/dist/pinia.mjs`).href

const storage = new Map()
globalThis.localStorage = {
  getItem: key => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
  removeItem: key => storage.delete(key),
}
globalThis.document = { documentElement: { lang: '' }, addEventListener() {}, removeEventListener() {}, hidden: false }
globalThis.window = { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) }

// Relative imports cannot resolve from a data: module, so each one is rewritten.
const translatorUrl = dataModule(transpile(read('src/i18n/translator.ts')))
const i18nUrl = dataModule(`export { isMessage } from '${translatorUrl}'`)
const tauriUrl = dataModule('export const isTauri = () => globalThis.__desktop ?? false; export const invoke = (...args) => globalThis.__invoke(...args);')
const scanCacheUrl = dataModule(transpile(read('src/stores/scanCache.ts').replaceAll("from '../i18n/translator'", `from '${translatorUrl}'`)))
const workspaceUrl = dataModule(transpile(read('src/stores/workspace.ts')
  .replaceAll("from 'vue'", `from '${vueUrl}'`)
  .replaceAll("from 'pinia'", `from '${piniaUrl}'`)
  .replaceAll("from '@tauri-apps/api/core'", `from '${tauriUrl}'`)
  .replaceAll("from '../i18n'", `from '${i18nUrl}'`)
  .replaceAll("from './scanCache'", `from '${scanCacheUrl}'`)
  .replaceAll('import.meta.env.DEV', 'false')))
const { useWorkspace } = await import(workspaceUrl)
const { readScan } = await import(scanCacheUrl)

const ROOT = 'D:/dev'
const git = { branch: 'main', commit: 'abc1234', dirty: false, changedFiles: 0 }
const pkg = {
  packageName: 'acme/bundle', constraint: '^1.0', dependencyType: 'require',
  localProjectId: 'bundle', localPath: `${ROOT}/bundle`, vendorPath: `${ROOT}/shop/vendor/acme/bundle`,
  backupPath: `${ROOT}/shop/vendor/.symfolinker/acme/bundle`,
  mode: 'local', linkStatus: 'linked', backupStatus: 'available', git,
  localVersion: null, constraintStatus: 'unknown', autoloadStatus: 'unknown', relinkable: false,
}
const scanResult = () => ({
  developmentRoot: ROOT,
  projects: [
    { id: 'shop', name: 'shop', group: null, composerName: 'acme/shop', path: `${ROOT}/shop`, git, composeFile: null, packages: [pkg] },
    { id: 'bundle', name: 'bundle', group: null, composerName: 'acme/bundle', path: `${ROOT}/bundle`, git, composeFile: null, packages: [] },
  ],
  warnings: [{ key: '{path} is unreadable; package statuses may be incomplete.', params: { path: ROOT } }],
  ambiguous: [],
})

// A fresh store setup is what happens when the app is reopened.
const opened = []
function reopen() {
  setActivePinia(createPinia())
  const store = useWorkspace()
  opened.push(store)
  return store
}

// Replaces the scan the way scan() and setMode() do: timestamp first, then the data.
async function storeScan(workspace, result, scannedAt = Date.now()) {
  workspace.scannedAt = scannedAt
  workspace.result = result
  await nextTick()
}

test.beforeEach(() => {
  globalThis.__desktop = false
  globalThis.__invoke = async () => { throw new Error('Unexpected IPC call') }
  storage.clear()
  storage.set('symfolinker.root', ROOT)
})
test.afterEach(() => { for (const store of opened.splice(0)) store.$dispose() })

test('a scan survives closing the app, with its timestamp and selected project', async () => {
  const first = reopen()
  await storeScan(first, scanResult(), 1700000000000)
  first.selectedId = 'bundle'
  await nextTick()

  const restored = reopen()
  assert.equal(restored.result?.developmentRoot, ROOT)
  assert.deepEqual(restored.result?.projects.map(project => project.id), ['shop', 'bundle'])
  assert.equal(restored.result?.projects[0].packages[0].packageName, 'acme/bundle')
  assert.deepEqual(restored.result?.warnings[0].params, { path: ROOT })
  assert.equal(restored.scannedAt, 1700000000000)
  assert.equal(restored.fromCache, true)
  assert.equal(restored.selectedId, 'bundle')
  assert.equal(restored.selected?.name, 'bundle')
})

// A development root grouped by team or client is restored the way it was scanned:
// the picker's groups and the filter both follow the directories on disk.
test('a grouped workspace keeps its groups through a restore, and the filter matches them', async () => {
  const project = (id, group, packages = []) => ({
    id, group, name: id.split('/').pop(), composerName: `acme/${id.split('/').pop()}`,
    path: `${ROOT}/${id}`, git, composeFile: null, packages,
  })
  const grouped = {
    developmentRoot: ROOT,
    projects: [
      project('apps/shop', 'apps', [{ ...pkg, localProjectId: 'library/bundle', localPath: `${ROOT}/library/bundle` }]),
      project('library/bundle', 'library'),
      project('tools', null),
    ],
    warnings: [],
    ambiguous: [],
  }
  const first = reopen()
  await storeScan(first, grouped)
  first.selectedId = 'library/bundle'
  await nextTick()

  const restored = reopen()
  assert.equal(restored.selectedId, 'library/bundle')
  assert.deepEqual(
    restored.projectGroups.map(entry => [entry.group, entry.projects.map(item => item.id)]),
    [['apps', ['apps/shop']], ['library', ['library/bundle']], ['', ['tools']]],
  )
  // The group is part of the id, so filtering on it narrows the picker to that team.
  restored.search = 'library'
  assert.deepEqual(restored.projects.map(item => item.id), ['library/bundle'])
})

test('a selection the restored scan no longer contains falls back to the first project', async () => {
  const first = reopen()
  await storeScan(first, scanResult())
  first.selectedId = 'bundle'
  await nextTick()
  await storeScan(first, { ...scanResult(), projects: scanResult().projects.filter(project => project.id !== 'bundle') })

  assert.equal(reopen().selectedId, 'shop')
})

test('a scan taken in another development root is not restored', async () => {
  await storeScan(reopen(), scanResult())
  storage.set('symfolinker.root', 'D:\other')

  const restored = reopen()
  assert.equal(restored.result, null)
  assert.equal(restored.scannedAt, 0)
  assert.equal(restored.fromCache, false)
})

test('an unreadable, incomplete or outdated cache entry is discarded instead of rendered', () => {
  const valid = { version: 4, scannedAt: 1, result: scanResult() }
  const withPackages = packages => ({
    ...scanResult(),
    projects: [{ ...scanResult().projects[0], packages }],
  })
  const rejected = [
    'not json at all',
    // Newer than this version understands, and two older shapes. Version 3 is the
    // one without the ambiguous-package list, so it predates duplicate checkouts
    // being answerable and must be re-read rather than rendered.
    JSON.stringify({ ...valid, version: 5 }),
    JSON.stringify({ ...valid, version: 3 }),
    JSON.stringify({ ...valid, version: 2 }),
    JSON.stringify({ ...valid, scannedAt: 0 }),
    JSON.stringify({ ...valid, result: { ...scanResult(), projects: [{ id: 'shop' }] } }),
    // A project cached before groups existed, and one with the wrong type in the
    // field: both describe projects this version would place in the wrong group.
    JSON.stringify({ ...valid, result: { ...scanResult(), projects: [{ ...scanResult().projects[0], group: undefined }] } }),
    JSON.stringify({ ...valid, result: { ...scanResult(), projects: [{ ...scanResult().projects[0], group: 7 }] } }),
    // A package without the fields this version renders, or with the wrong type in
    // one of them, describes a different shape than the interface expects.
    JSON.stringify({ ...valid, result: withPackages([{ ...pkg, relinkable: undefined }]) }),
    JSON.stringify({ ...valid, result: withPackages([{ ...pkg, relinkable: 'yes' }]) }),
    JSON.stringify({ ...valid, result: withPackages([{ ...pkg, constraintStatus: 7 }]) }),
    JSON.stringify({ ...valid, result: withPackages([{ ...pkg, localVersion: 2 }]) }),
    JSON.stringify({ ...valid, result: { ...scanResult(), warnings: ['plain string'] } }),
    JSON.stringify({ ...valid, result: { ...scanResult(), developmentRoot: 42 } }),
    // A duplicate-checkout question whose candidates cannot be rendered, or whose
    // answer is the wrong type. Offering the wrong checkout is worse than rescanning.
    JSON.stringify({ ...valid, result: { ...scanResult(), ambiguous: [{ packageName: 'acme/bundle' }] } }),
    JSON.stringify({ ...valid, result: { ...scanResult(), ambiguous: [{ packageName: 'acme/bundle', candidates: [{ projectId: 'bundle-b' }], chosen: null }] } }),
    JSON.stringify({ ...valid, result: { ...scanResult(), ambiguous: [{ packageName: 'acme/bundle', candidates: [], chosen: 7 }] } }),
  ]
  for (const entry of rejected) {
    storage.set('symfolinker.scan', entry)
    assert.equal(readScan(ROOT), null, entry.slice(0, 60))
    assert.equal(reopen().result, null, entry.slice(0, 60))
  }
  storage.set('symfolinker.scan', JSON.stringify(valid))
  assert.equal(readScan(ROOT)?.result.developmentRoot, ROOT)
})

test('clearing the saved scan empties both the interface and the stored copy', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  assert.ok(storage.has('symfolinker.scan'))

  workspace.forgetScan()
  await nextTick()
  assert.equal(workspace.result, null)
  assert.equal(workspace.scannedAt, 0)
  assert.equal(storage.has('symfolinker.scan'), false)
  assert.equal(reopen().result, null)
})

test('a scan without a development root reports an error and stores nothing', async () => {
  storage.set('symfolinker.root', '')
  const workspace = reopen()
  await workspace.scan()
  await nextTick()

  assert.equal(workspace.result, null)
  assert.ok(workspace.error)
  assert.equal(storage.has('symfolinker.scan'), false)
})

/// Answers every mutating command with a scan, and records what was asked.
function recordingBackend(packages = { 'acme/bundle': 'local' }) {
  const calls = []
  globalThis.__desktop = true
  globalThis.__invoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'sync_container_mounts') return { services: [], phpService: null, suggested: false, volumes: [], projectContainerPath: null, checks: [], message: null, synced: null }
    const result = scanResult()
    return {
      ...result,
      projects: result.projects.map(project => ({
        ...project,
        packages: project.packages.map(entry => ({ ...entry, mode: packages[entry.packageName] ?? entry.mode, relinkable: false })),
      })),
    }
  }
  return calls
}

// The autoload comparison can only report; this is the repair. It has to say where
// Composer ran, and it has to read the workspace again afterwards, because the label
// it clears is read from disk.
test('regenerating the autoloader reports where Composer ran and rescans afterwards', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  const calls = []
  globalThis.__desktop = true
  globalThis.__invoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'dump_autoload') {
      return { location: 'container', service: 'php-fpm', output: 'Generated autoload files', message: null }
    }
    return scanResult()
  }

  await workspace.dumpAutoload()

  assert.equal(workspace.autoloadDump?.location, 'container')
  assert.equal(workspace.autoloadDump?.service, 'php-fpm')
  assert.equal(workspace.autoloadError, '')
  assert.deepEqual(calls[0], { command: 'dump_autoload', args: { developmentRoot: ROOT, projectId: 'shop' } })
  assert.ok(calls.some(call => call.command === 'scan_projects'), 'the workspace is read again')
})

test('a failed regeneration keeps the backend message and changes nothing else', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'Composer error: {detail}', params: { detail: 'boom' } } }

  await workspace.dumpAutoload()

  assert.deepEqual(workspace.autoloadError, { key: 'Composer error: {detail}', params: { detail: 'boom' } })
  assert.equal(workspace.autoloadDump, null)
  assert.equal(workspace.result?.projects.length, 2)
})

// A button may only promise a program this computer has, and opening one names a
// project rather than a path: the backend resolves the directory itself.
test('only the tools the backend found are offered, and opening one names the project', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'bundle'
  const calls = []
  globalThis.__desktop = true
  globalThis.__invoke = async (command, args) => {
    calls.push({ command, args })
    if (command === 'external_tools') return ['vscode', 'files']
    return undefined
  }

  await workspace.loadExternalTools()
  assert.deepEqual(workspace.externalTools, ['vscode', 'files'])

  await workspace.openExternal('vscode')
  assert.deepEqual(calls.at(-1), {
    command: 'open_external',
    args: { developmentRoot: ROOT, projectId: 'bundle', tool: 'vscode' },
  })
  assert.equal(workspace.externalError, '')
})

test('a tool that cannot start is reported without breaking the workspace', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'This tool could not be found on this computer. Is it in PATH?', params: {} } }

  await workspace.openExternal('phpstorm')

  assert.equal(workspace.externalError.key, 'This tool could not be found on this computer. Is it in PATH?')
  assert.equal(workspace.result?.projects.length, 2)
})

// Without a desktop backend there is nothing to watch, and that must stay a quiet
// fallback rather than an error: the five-minute poll is still there.
test('watching stays off when the backend refuses it', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'Changes to this project cannot be watched; status is polled instead.', params: {} } }

  await workspace.startWatching()

  assert.equal(workspace.watching, false)
  assert.equal(workspace.error, '')
})

test('a switch records what would put it back, and the undo runs as a profile', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  const calls = recordingBackend()

  await workspace.setMode(workspace.selected.packages[0], 'vendor')

  assert.deepEqual(workspace.lastSwitch, {
    label: 'acme/bundle',
    entries: [{ projectId: 'shop', packageName: 'acme/bundle', mode: 'local' }],
  }, 'the mode before the switch is what an undo restores')

  await workspace.undoSwitch()

  const undo = calls.find(call => call.command === 'apply_profile')
  assert.deepEqual(undo?.args.entries, [{ projectId: 'shop', packageName: 'acme/bundle', mode: 'local' }])
  assert.equal(workspace.lastSwitch, null, 'an undo cannot itself be undone')
})

test('a failed switch leaves nothing to undo', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'The switch could not be completed.' } }

  await workspace.setMode(workspace.selected.packages[0], 'vendor')

  assert.ok(workspace.swapError)
  assert.equal(workspace.lastSwitch, null)
})

test('links replaced outside the app are offered for restoring across projects', async () => {
  const workspace = reopen()
  const replaced = scanResult()
  await storeScan(workspace, {
    ...replaced,
    projects: replaced.projects.map(project => ({
      ...project,
      packages: project.packages.map(entry => ({ ...entry, mode: 'vendor', relinkable: true })),
    })),
  })
  workspace.selectedId = 'shop'
  const calls = recordingBackend()

  assert.deepEqual(workspace.relinkablePackages, [
    { projectId: 'shop', projectName: 'shop', packageName: 'acme/bundle' },
  ])

  await workspace.relinkPackages()

  const relink = calls.find(call => call.command === 'relink_packages')
  assert.equal(relink?.args.projectId, null, 'the whole workspace is restored at once')
  assert.deepEqual(workspace.lastSwitch?.entries, [{ projectId: 'shop', packageName: 'acme/bundle', mode: 'vendor' }])
})

test('preserved versions and write checks do not outlive the project they describe', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async command => {
    if (command === 'project_history') return [{ packageName: 'acme/bundle', archiveId: '1-2-3', path: `${ROOT}/shop/vendor/.symfolinker/.history/acme/bundle/1-2-3`, version: '1.0.0', archivedAt: 1700000000, sizeBytes: 2048, files: 3, recognized: true, directory: true }]
    if (command === 'preflight_checks') return { rootWritable: true, vendorWritable: true, linkKind: 'symlink', freeBytes: 1024, preservedBytes: 2048, message: null }
    throw new Error(`Unexpected command ${command}`)
  }

  await workspace.loadArchives()
  await workspace.runPreflight()
  assert.equal(workspace.archives.length, 1)
  assert.equal(workspace.preflight?.linkKind, 'symlink')

  workspace.selectedId = 'bundle'
  assert.deepEqual(workspace.archives, [], 'another project has its own history')
  assert.equal(workspace.preflight, null)
})

function deferred() {
  let resolve, reject
  const promise = new Promise((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}
const liveStatus = branch => ({ git: { ...git, branch }, docker: { available: true, services: [], composeFile: null, engine: 'Docker', message: null } })

test('switching projects starts a new status request and ignores the old response', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  globalThis.__desktop = true
  const requests = []
  globalThis.__invoke = (command, args) => {
    assert.equal(command, 'project_status')
    const response = deferred()
    requests.push({ args, ...response })
    return response.promise
  }
  workspace.selectedId = 'shop'
  workspace.selectedId = 'bundle'
  assert.equal(requests.length, 2)
  requests[1].resolve(liveStatus('bundle-branch'))
  await nextTick()
  requests[0].resolve(liveStatus('shop-branch'))
  await nextTick()
  assert.equal(workspace.status.git.branch, 'bundle-branch')
  assert.equal(workspace.statusBusy, false)
  assert.equal(requests.length, 2, 'applying Git does not trigger a new poll')
})

test('container responses cannot leak across a project switch, including A to B to A', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  const old = deferred(), fresh = deferred()
  let count = 0
  globalThis.__invoke = command => command === 'inspect_container' ? (++count === 1 ? old.promise : fresh.promise) : Promise.resolve(liveStatus('main'))
  workspace.container = { phpService: 'old' }
  const first = workspace.inspectContainer()
  workspace.selectedId = 'bundle'
  assert.equal(workspace.container, null)
  assert.equal(workspace.containerBusy, false)
  workspace.selectedId = 'shop'
  const second = workspace.inspectContainer()
  fresh.resolve({ phpService: 'fresh' })
  await second
  old.resolve({ phpService: 'stale' })
  await first
  assert.equal(workspace.container.phpService, 'fresh')
})

test('a service save failure is shown in the container panel', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'The PHP service could not be saved.' } }
  await workspace.selectPhpService('php')
  assert.equal(workspace.containerError.key, 'The PHP service could not be saved.')
  assert.equal(workspace.containerBusy, false)
})

test('an uncertain swap with failed refresh keeps the workspace visible and blocks further mutations', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'The switch succeeded, but the workspace could not be read back. Rescan to continue.' } }
  await workspace.setMode(pkg, 'vendor')
  await nextTick()
  assert.equal(workspace.selectedId, 'shop')
  assert.equal(workspace.result.projects.length, 2)
  assert.equal(workspace.mutationBlocked, true)
  assert.equal(storage.has('symfolinker.scan'), false)
  assert.match(workspace.swapError.key, /switch succeeded/)
})

test('profiles persist by workspace and replace an existing name', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  assert.equal(workspace.saveProfile('Development'), true)
  assert.equal(workspace.saveProfile('Development'), true)
  assert.equal(workspace.workspaceProfiles.length, 1)
  const restored = reopen()
  assert.equal(restored.workspaceProfiles[0].entries[0].mode, 'local')
  assert.equal(restored.saveProfile('Cached'), false)
  restored.removeProfile('Development')
  assert.equal(reopen().workspaceProfiles.length, 0)
})

test('successful scans remember workspaces and opening another workspace clears old data', async () => {
  const workspace = reopen()
  globalThis.__desktop = true
  globalThis.__invoke = async command => command === 'scan_projects' ? scanResult() : liveStatus('main')
  await workspace.scan()
  assert.equal(workspace.savedWorkspaces[0].root, ROOT)
  const pending = deferred()
  globalThis.__invoke = () => pending.promise
  const opening = workspace.openWorkspace('D:/other')
  assert.equal(workspace.result, null)
  pending.resolve({ developmentRoot: 'D:/other', projects: [], warnings: [] })
  await opening
  assert.equal(workspace.savedWorkspaces.length, 2)
  workspace.removeWorkspace(ROOT)
  assert.equal(reopen().savedWorkspaces.length, 1)
})


test('a refused swap refreshes package modes without losing the selected workspace', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  const calls = []
  const next = scanResult()
  next.projects[0].packages = [{ ...pkg, mode: 'vendor', linkStatus: 'notLinked' }]
  globalThis.__invoke = async command => {
    calls.push(command)
    if (command === 'activate_local') throw { key: 'A backup already exists at {path}. SymfoLinker will not overwrite it.', params: { path: pkg.backupPath } }
    if (command === 'scan_projects') return next
    return liveStatus('main')
  }
  await workspace.setMode(pkg, 'local')
  assert.equal(workspace.selectedId, 'shop')
  assert.equal(workspace.selected.packages[0].mode, 'vendor')
  assert.equal(workspace.mutationBlocked, false)
  assert.deepEqual(calls.slice(0,2), ['activate_local', 'scan_projects'])
  assert.match(workspace.swapError.key, /backup already exists/)
})

test('container mount application is scoped to the chosen service and keeps the workspace on failure', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  workspace.container = { phpService: 'php-fpm' }
  globalThis.__desktop = true
  let request
  globalThis.__invoke = async (command, args) => {
    request = { command, args }
    throw { key: 'The container could not be inspected.' }
  }
  await workspace.applyContainerMounts()
  assert.equal(request.command, 'apply_container_mounts')
  assert.equal(request.args.projectId, 'shop')
  assert.equal(request.args.service, 'php-fpm')
  assert.equal(workspace.selectedId, 'shop')
  assert.equal(workspace.result.projects.length, 2)
  assert.equal(workspace.containerBusy, false)
  assert.equal(workspace.containerError.key, 'The container could not be inspected.')
})

// The container report a synchronisation returns; only the fields the store reads.
const syncReport = synced => ({
  services: ['php-fpm'], phpService: 'php-fpm', suggested: false, volumes: [],
  projectContainerPath: null, checks: [], message: null, mountPlan: [], canApplyMounts: true, synced,
})

test('switching to vendor synchronises the container mounts for that project', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  const switched = scanResult()
  switched.projects[0].packages = [{ ...pkg, mode: 'vendor', linkStatus: 'notLinked' }]
  const calls = []
  globalThis.__invoke = async (command, args) => {
    calls.push([command, args && args.projectId])
    if (command === 'activate_vendor') return switched
    if (command === 'sync_container_mounts') {
      return syncReport({
        added: [],
        removed: [{ packageName: 'acme/bundle', hostPath: ROOT + '/bundle', containerPath: '/bundle' }],
        recreated: true,
      })
    }
    return liveStatus('main')
  }

  await workspace.setMode(pkg, 'vendor')
  await workspace.flushMountSync()

  assert.ok(calls.some(call => call[0] === 'sync_container_mounts' && call[1] === 'shop'))
  assert.equal(workspace.selected.packages[0].mode, 'vendor')
  assert.equal(workspace.mountSync.removed.length, 1)
  assert.equal(workspace.mountSync.removed[0].containerPath, '/bundle')
  assert.equal(workspace.mountSync.recreated, true)
  assert.equal(workspace.mountSyncBusy, '')
  assert.equal(workspace.containerBusy, false)
  assert.equal(workspace.container.phpService, 'php-fpm')
})

test('a failing mount synchronisation is reported without undoing the switch', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  const switched = scanResult()
  switched.projects[0].packages = [{ ...pkg, mode: 'vendor', linkStatus: 'notLinked' }]
  globalThis.__invoke = async command => {
    if (command === 'activate_vendor') return switched
    if (command === 'sync_container_mounts') throw { key: 'The container mounts could not be synchronised.' }
    return liveStatus('main')
  }

  await workspace.setMode(pkg, 'vendor')
  await workspace.flushMountSync()

  assert.equal(workspace.swapError, '')
  assert.equal(workspace.selected.packages[0].mode, 'vendor')
  assert.equal(workspace.mountSyncError.key, 'The container mounts could not be synchronised.')
  assert.equal(workspace.mountSyncBusy, '')
  assert.equal(workspace.containerBusy, false)
  assert.equal(workspace.mutationBlocked, false)
})

test('a profile synchronises every project it switched, once each', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  globalThis.__desktop = true
  const synced = []
  globalThis.__invoke = async (command, args) => {
    if (command === 'apply_profile') return scanResult()
    if (command === 'sync_container_mounts') {
      synced.push(args.projectId)
      return syncReport({ added: [], removed: [], recreated: false })
    }
    return liveStatus('main')
  }

  await workspace.applyProfile({
    name: 'daily', root: ROOT,
    entries: [
      { projectId: 'shop', packageName: 'acme/bundle', mode: 'local' },
      { projectId: 'shop', packageName: 'acme/other', mode: 'vendor' },
      { projectId: 'bundle', packageName: 'acme/deep', mode: 'local' },
    ],
  })
  await workspace.flushMountSync()

  assert.deepEqual(synced, ['shop', 'bundle'])
  assert.equal(workspace.mountSyncBusy, '')
})

test('switches in quick succession are synchronised in a single pass', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  let syncs = 0
  globalThis.__invoke = async command => {
    if (command === 'activate_local' || command === 'activate_vendor') return scanResult()
    if (command === 'sync_container_mounts') {
      syncs += 1
      return syncReport({ added: [], removed: [], recreated: false })
    }
    return liveStatus('main')
  }

  await workspace.setMode(pkg, 'vendor')
  await workspace.setMode(pkg, 'local')
  assert.equal(syncs, 0, 'nothing is recreated while switches are still arriving')

  await workspace.flushMountSync()
  assert.equal(syncs, 1, 'both switches share one container recreation')
})

test('the project filter also matches the packages a project requires', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())

  workspace.search = 'shop'
  assert.deepEqual(workspace.projects.map(project => project.id), ['shop'])

  // The bundle is required by shop and is itself a project, so both are reachable.
  workspace.search = 'acme/bundle'
  assert.deepEqual(workspace.projects.map(project => project.id), ['shop', 'bundle'])

  workspace.search = 'nothing-like-this'
  assert.deepEqual(workspace.projects, [])
})
