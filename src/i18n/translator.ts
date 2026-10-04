export type Locale = 'en' | 'nl' | 'fr' | 'de'
export interface Message { key: string; params?: Record<string, string | number> }
export type Catalog = Record<string, string>

export const languages: { code: Locale; label: string }[] = [
  { code: 'en', label: 'English (EN)' },
  { code: 'nl', label: 'Nederlands (NL)' },
  { code: 'fr', label: 'Français (FR)' },
  { code: 'de', label: 'Deutsch (DE)' },
]

export function resolveLocale(value: string | null): Locale {
  return languages.find(language => language.code === value)?.code ?? 'en'
}

// Replace placeholders in one pass: interpolated paths and external output are data.
export function translate(catalog: Catalog, fallback: Catalog, key: string, params: Message['params'] = {}): string {
  const template = Object.hasOwn(catalog, key) ? catalog[key] : Object.hasOwn(fallback, key) ? fallback[key] : key
  return template.replace(/\{(\w+)\}/g, (placeholder, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : placeholder)
}

export function isMessage(value: unknown): value is Message {
  if (!value || typeof value !== 'object' || !('key' in value) || typeof value.key !== 'string') return false
  if (!('params' in value) || value.params === undefined) return true
  return value.params !== null && typeof value.params === 'object' && !Array.isArray(value.params)
    && Object.values(value.params).every(item => typeof item === 'string' || typeof item === 'number')
}
