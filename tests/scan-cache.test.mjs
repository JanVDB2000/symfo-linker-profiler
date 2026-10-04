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

const ROOT = 'D:\dev'
const git = { branch: 'main', commit: 'abc1234', dirty: false, changedFiles: 0 }
const pkg = {
  packageName: 'acme/bundle', constraint: '^1.0', dependencyType: 'require',
  localProjectId: 'bundle', localPath: `${ROOT}\bundle`, vendorPath: `${ROOT}\shop\vendor\acme\bundle`,
  backupPath: `${ROOT}\shop\vendor\acme\bundle.symfolinker-backup`,
  mode: 'local', linkStatus: 'linked', backupStatus: 'available', git,
}
const scanResult = () => ({
  developmentRoot: ROOT,
  projects: [
    { id: 'shop', name: 'shop', composerName: 'acme/shop', path: `${ROOT}\shop`, git, composeFile: null, packages: [pkg] },
    { id: 'bundle', name: 'bundle', composerName: 'acme/bundle', path: `${ROOT}\bundle`, git, composeFile: null, packages: [] },
  ],
  warnings: [{ key: '{path} is unreadable; package statuses may be incomplete.', params: { path: ROOT } }],
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
  const valid = { version: 1, scannedAt: 1, result: scanResult() }
  const rejected = [
    'not json at all',
    JSON.stringify({ ...valid, version: 2 }),
    JSON.stringify({ ...valid, scannedAt: 0 }),
    JSON.stringify({ ...valid, result: { ...scanResult(), projects: [{ id: 'shop' }] } }),
    JSON.stringify({ ...valid, result: { ...scanResult(), warnings: ['plain string'] } }),
    JSON.stringify({ ...valid, result: { ...scanResult(), developmentRoot: 42 } }),
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

test('an uncertain swap result clears the scan and cache while retaining the error', async () => {
  const workspace = reopen()
  await storeScan(workspace, scanResult())
  workspace.selectedId = 'shop'
  globalThis.__desktop = true
  globalThis.__invoke = async () => { throw { key: 'The switch succeeded, but the workspace could not be read back. Rescan to continue.' } }
  await workspace.setMode(pkg, 'vendor')
  await nextTick()
  assert.equal(workspace.result, null)
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
