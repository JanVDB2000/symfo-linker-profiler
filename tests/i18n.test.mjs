import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'
import test from 'node:test'
import ts from 'typescript'
import { parse, compileScript } from '@vue/compiler-sfc'
import { createSSRApp } from 'vue'
import { createPinia, setActivePinia } from 'pinia'
import { renderToString } from '@vue/server-renderer'

const read = path => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8')
const catalogs = Object.fromEntries(['en', 'nl', 'fr', 'de'].map(code => [code, JSON.parse(read(`src/i18n/locales/${code}.json`))]))
const dataModule = source => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
const transpile = source => ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
const packageUrl = name => pathToFileURL(`${process.cwd()}/node_modules/${name}/index.mjs`).href
const vueUrl = packageUrl('vue')
const piniaUrl = pathToFileURL(`${process.cwd()}/node_modules/pinia/dist/pinia.mjs`).href
const translatorUrl = dataModule(transpile(read('src/i18n/translator.ts')))
const { translate, resolveLocale, isMessage } = await import(translatorUrl)
const storage = new Map()
globalThis.localStorage = { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) }
globalThis.document = { documentElement: { lang: '' }, addEventListener() {}, hidden: false }
globalThis.window = { matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) }
let i18nSource = read('src/i18n/index.ts').replaceAll("from 'vue'", `from '${vueUrl}'`).replaceAll("from './translator'", `from '${translatorUrl}'`)
for (const code of Object.keys(catalogs)) i18nSource = i18nSource.replace(`import ${code} from './locales/${code}.json'`, `const ${code} = ${JSON.stringify(catalogs[code])}`)
const i18nUrl = dataModule(transpile(i18nSource))
const i18n = await import(i18nUrl)
const tauriUrl = dataModule('export const isTauri = () => false; export const invoke = async () => { throw new Error("Unexpected IPC call") }; export const open = async () => null;')
const scanCacheUrl = dataModule(transpile(read('src/stores/scanCache.ts').replaceAll("from '../i18n/translator'", `from '${translatorUrl}'`)))
const workspaceUrl = dataModule(transpile(read('src/stores/workspace.ts')
  .replaceAll("from 'vue'", `from '${vueUrl}'`)
  .replaceAll("from 'pinia'", `from '${piniaUrl}'`)
  .replaceAll("from '@tauri-apps/api/core'", `from '${tauriUrl}'`)
  .replaceAll("from '../i18n'", `from '${i18nUrl}'`)
  .replaceAll("from './scanCache'", `from '${scanCacheUrl}'`)
  .replaceAll('import.meta.env.DEV', 'false')))
const { useWorkspace } = await import(workspaceUrl)
function vueModule(path, id, replacements = {}, transformSource = source => source) {
  const { descriptor } = parse(transformSource(read(path)), { filename: path })
  let source = compileScript(descriptor, { id, inlineTemplate: true }).content
  for (const [from, to] of Object.entries({ vue: vueUrl, ...replacements })) source = source.replaceAll(`from '${from}'`, `from '${to}'`).replaceAll(`from "${from}"`, `from '${to}'`)
  return dataModule(transpile(source))
}
const iconUrl = vueModule('src/components/ProfilerIcon.vue', 'icon')
const spinnerUrl = vueModule('src/components/Spinner.vue', 'spinner')
const appUrl = vueModule('src/App.vue', 'app', {
  '@tauri-apps/plugin-dialog': tauriUrl, '@tauri-apps/api/core': tauriUrl,
  './stores/workspace': workspaceUrl, './components/ProfilerIcon.vue': iconUrl,
  './components/Spinner.vue': spinnerUrl, './i18n': i18nUrl,
})
const { default: App } = await import(appUrl)

const placeholders = text => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort()

test('all four catalogs have identical keys and interpolation parameters', () => {
  const keys = Object.keys(catalogs.en).sort()
  for (const [code, catalog] of Object.entries(catalogs)) {
    assert.deepEqual(Object.keys(catalog).sort(), keys, code)
    for (const key of keys) {
      assert.ok(catalog[key].trim(), `${code}: ${key}`)
      assert.deepEqual(placeholders(catalog[key]), placeholders(catalogs.en[key]), `${code}: ${key}`)
      assert.ok(!catalog[key].includes('\uFFFD'), `${code}: invalid encoding in ${key}`)
      if (!key.includes('?')) assert.ok(!catalog[key].includes('?'), `${code}: corrupted characters in ${key}`)
    }
  }
})

test('all literal frontend and backend message keys exist in the catalogs', () => {
  for (const path of ['src/App.vue', 'src/stores/workspace.ts', 'src/dev/fixture.ts', 'src-tauri/src/discovery.rs', 'src-tauri/src/runtime.rs', 'src-tauri/src/container.rs', 'src-tauri/src/lib.rs']) {
    const source = read(path)
    const patterns = [ /\bt\('((?:\\.|[^'\\])*)'/g, /key: '((?:\\.|[^'\\])*)'/g, /Message::(?:new|with)\(\s*"((?:\\.|[^"\\])*)"/g ]
    for (const pattern of patterns) for (const match of source.matchAll(pattern)) {
      const key = match[1].replaceAll('\\\\', '\\').replaceAll("\\'", "'")
      assert.ok(Object.hasOwn(catalogs.en, key), `${path}: missing ${key}`)
    }
  }
})

test('English is the default, invalid saved locales fall back to English', () => {
  assert.equal(resolveLocale(null), 'en')
  assert.equal(resolveLocale('es'), 'en')
  for (const code of Object.keys(catalogs)) assert.equal(resolveLocale(code), code)
})

test('interpolation preserves paths and never substitutes inside parameter data', () => {
  const path = 'D:\\dev\\{name}\\bundle $& <script>'
  assert.equal(translate(catalogs.de, catalogs.en, '{path} does not contain composer.json.', { path }), `${path} enthält keine composer.json.`)
  assert.equal(translate({}, catalogs.en, '{count} changed files', { count: 0 }), 'Changed files: 0')
  assert.equal(translate({}, {}, 'Unknown {key}', { key: 'value' }), 'Unknown value')
  assert.equal(translate({}, {}, '{missing}'), '{missing}')
  assert.equal(isMessage({ key: 'test', params: { count: 1 } }), true)
  assert.equal(isMessage({ key: 'test', params: null }), false)
  assert.equal(isMessage({ key: 'test', params: { bad: {} } }), false)
})

test('changing language updates existing messages, document language and persistence', () => {
  const message = { key: '{name}: composer.json is invalid or unreadable; project skipped.', params: { name: 'my-project' } }
  for (const code of Object.keys(catalogs)) {
    i18n.locale.value = code
    assert.equal(i18n.t('Projects'), catalogs[code].Projects)
    assert.equal(i18n.translateMessage(message), catalogs[code][message.key].replace('{name}', 'my-project'))
    assert.equal(document.documentElement.lang, code)
    assert.equal(storage.get('symfolinker.language'), code)
  }
})

test('the rendered app translates navigation, warnings and errors in every language', async () => {
  for (const code of Object.keys(catalogs)) {
    i18n.locale.value = code
    const pinia = createPinia()
    setActivePinia(pinia)
    const workspace = useWorkspace()
    workspace.error = { key: 'Choose a development root first.' }
    workspace.result = { developmentRoot: 'D:\\dev', projects: [], warnings: [{ key: '{path} is unreadable; package statuses may be incomplete.', params: { path: '<unsafe>' } }] }
    const html = await renderToString(createSSRApp(App).use(pinia))
    assert.ok(html.includes(catalogs[code].Projects), code)
    assert.ok(html.includes(catalogs[code].Settings), code)
    assert.ok(html.includes(catalogs[code]['Choose a development root first.']), code)
    assert.ok(html.includes('&lt;unsafe&gt;'), 'parameters must be escaped by Vue')
    assert.ok(!html.includes('<unsafe>'))
  }
})

test('Settings exposes a language control with all four options', () => {
  const source = read('src/App.vue')
  assert.match(source, /tab === 'Settings'/)
  assert.match(source, /<label for="language">\{\{ t\('Language'\) \}\}<\/label>/)
  assert.match(source, /<select id="language" v-model="locale">/)
  assert.deepEqual(i18n.languages.map(language => language.code), ['en', 'nl', 'fr', 'de'])
})


test('the Settings panel renders a translated language selector in every language', async () => {
  const settingsUrl = vueModule('src/App.vue', 'settings', {
    '@tauri-apps/plugin-dialog': tauriUrl, '@tauri-apps/api/core': tauriUrl,
    './stores/workspace': workspaceUrl, './components/ProfilerIcon.vue': iconUrl,
  './components/Spinner.vue': spinnerUrl, './i18n': i18nUrl,
  }, source => source.replace("const tab = ref('Projects')", "const tab = ref('Settings')"))
  const { default: SettingsApp } = await import(settingsUrl)
  for (const code of Object.keys(catalogs)) {
    i18n.locale.value = code
    const html = await renderToString(createSSRApp(SettingsApp).use(createPinia()))
    assert.ok(html.includes(`<label for="language">${catalogs[code].Language}</label>`))
    assert.ok(html.includes('<select id="language">'))
    for (const language of i18n.languages) {
      assert.ok(html.includes(`value="${language.code}"`))
      assert.ok(html.includes(language.label))
    }
    assert.ok(!html.includes('Instellingen') || code === 'nl')
  }
})

test('the saved language is restored when the translation module initializes', async () => {
  storage.set('symfolinker.language', 'fr')
  const restored = await import(dataModule(transpile(i18nSource + '\n// fresh initialization')))
  assert.equal(restored.locale.value, 'fr')
  assert.equal(restored.t('Projects'), 'Projets')
  assert.equal(document.documentElement.lang, 'fr')
})


test('count labels work for zero, one and multiple items in every language', () => {
  for (const code of Object.keys(catalogs)) {
    for (const count of [0, 1, 2]) {
      const label = translate(catalogs[code], catalogs.en, '{count} changed files', { count })
      const issues = translate(catalogs[code], catalogs.en, '{count} scan issue(s)', { count })
      assert.ok(label.endsWith(`: ${count}`), `${code}: ${label}`)
      assert.ok(issues.endsWith(`: ${count}`), `${code}: ${issues}`)
      assert.ok(!issues.includes('(s)') && !issues.includes('(en)') && !issues.includes('(e)'))
    }
  }
})
