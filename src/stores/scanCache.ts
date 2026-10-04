import { isMessage } from '../i18n/translator'
import type { ScanResult } from '../types'

const SCAN_KEY = 'symfolinker.scan'
const PROJECT_KEY = 'symfolinker.project'
// Bump this when the cached shape changes, so older entries are discarded instead of rendered.
const CACHE_VERSION = 1
// localStorage holds roughly 5 MB per origin. Staying well below that keeps a very
// large workspace from failing every later write with a quota error.
const MAX_CACHE_BYTES = 2_000_000

/** A scan restored from this computer, with the moment it was taken. */
export interface CachedScan { scannedAt: number; result: ScanResult }

const PROJECT_STRINGS = ['id', 'name', 'path']
const PACKAGE_STRINGS = ['packageName', 'constraint', 'dependencyType', 'localProjectId',
  'localPath', 'vendorPath', 'backupPath', 'mode', 'linkStatus', 'backupStatus']

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

function hasStrings(value: Record<string, unknown>, keys: string[]): boolean {
  return keys.every(key => typeof value[key] === 'string')
}

function isNullableString(value: unknown): boolean {
  return value === null || typeof value === 'string'
}

function isGitInfo(value: unknown): boolean {
  if (value === null) return true
  return isRecord(value) && hasStrings(value, ['branch', 'commit'])
    && typeof value.dirty === 'boolean' && typeof value.changedFiles === 'number'
}

function isPackageStatus(value: unknown): boolean {
  return isRecord(value) && hasStrings(value, PACKAGE_STRINGS) && isGitInfo(value.git)
}

function isProject(value: unknown): boolean {
  return isRecord(value) && hasStrings(value, PROJECT_STRINGS)
    && isNullableString(value.composerName) && isNullableString(value.composeFile)
    && isGitInfo(value.git) && Array.isArray(value.packages) && value.packages.every(isPackageStatus)
}

// The cache is editable outside the app, so it is validated like any other external
// input: one missing field discards the entry rather than reaching the interface.
function isScanResult(value: unknown): value is ScanResult {
  return isRecord(value) && typeof value.developmentRoot === 'string'
    && Array.isArray(value.projects) && value.projects.every(isProject)
    && Array.isArray(value.warnings) && value.warnings.every(isMessage)
}

/**
 * Reads the saved scan, but only for the root it was taken in: a cache from another
 * development root describes projects that are not the ones being opened.
 */
export function readScan(root: string): CachedScan | null {
  if (!root) return null
  const entry = parse(read(SCAN_KEY))
  if (!isRecord(entry) || entry.version !== CACHE_VERSION) return null
  const scannedAt = typeof entry.scannedAt === 'number' && entry.scannedAt > 0 ? entry.scannedAt : 0
  if (!scannedAt || !isScanResult(entry.result) || entry.result.developmentRoot !== root) return null
  return { scannedAt, result: entry.result }
}

export function writeScan(result: ScanResult, scannedAt: number): void {
  const payload = JSON.stringify({ version: CACHE_VERSION, scannedAt, result })
  // A workspace too large to store is not cacheable; drop the previous entry rather
  // than leaving an older scan behind as if it described what is on screen.
  if (payload.length > MAX_CACHE_BYTES) { clearScan(); return }
  write(SCAN_KEY, payload)
}

export function clearScan(): void { remove(SCAN_KEY) }

export function readSelected(): string { return read(PROJECT_KEY) ?? '' }

export function writeSelected(id: string): void { write(PROJECT_KEY, id) }

export interface ProfileEntry { projectId: string; packageName: string; mode: 'local' | 'vendor' }
export interface LinkProfile { name: string; root: string; entries: ProfileEntry[] }
export interface SavedWorkspace { root: string; name: string }

export function readWorkspaces(): SavedWorkspace[] {
  const value = parse(read('symfolinker.workspaces'))
  return Array.isArray(value) ? value.filter((item): item is SavedWorkspace =>
    isRecord(item) && typeof item.root === 'string' && !!item.root && typeof item.name === 'string') : []
}
export function writeWorkspaces(value: SavedWorkspace[]): void { write('symfolinker.workspaces', JSON.stringify(value)) }

export function readProfiles(): LinkProfile[] {
  const value = parse(read('symfolinker.profiles'))
  return Array.isArray(value) ? value.filter((item): item is LinkProfile =>
    isRecord(item) && typeof item.name === 'string' && !!item.name && typeof item.root === 'string'
    && Array.isArray(item.entries) && item.entries.length > 0 && item.entries.every(entry =>
      isRecord(entry) && typeof entry.projectId === 'string' && typeof entry.packageName === 'string'
      && (entry.mode === 'local' || entry.mode === 'vendor'))) : []
}
export function writeProfiles(value: LinkProfile[]): void { write('symfolinker.profiles', JSON.stringify(value)) }

// Storage can be full, disabled or absent. A cache that cannot be read or written is
// only a lost shortcut, so these report nothing: the caller scans instead.
function read(key: string): string | null {
  try { return localStorage.getItem(key) } catch { return null }
}

function write(key: string, value: string): void {
  try { localStorage.setItem(key, value) } catch { return }
}

function remove(key: string): void {
  try { localStorage.removeItem(key) } catch { return }
}

function parse(raw: string | null): unknown {
  if (!raw) return null
  try { return JSON.parse(raw) } catch { return null }
}
