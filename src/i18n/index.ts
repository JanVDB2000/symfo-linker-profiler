import { ref, watch } from 'vue'
import en from './locales/en.json'
import nl from './locales/nl.json'
import fr from './locales/fr.json'
import de from './locales/de.json'
import { resolveLocale, translate, isMessage } from './translator'
import type { Locale, Message, Catalog } from './translator'

export { languages, isMessage } from './translator'
export type { Message } from './translator'

const catalogs: Record<Locale, Catalog> = { en, nl, fr, de }
export const locale = ref<Locale>(resolveLocale(localStorage.getItem('symfolinker.language')))

watch(locale, value => {
  document.documentElement.lang = value
  localStorage.setItem('symfolinker.language', value)
}, { immediate: true, flush: 'sync' })

export function t(key: string, params?: Message['params']): string {
  return translate(catalogs[locale.value], en, key, params)
}

export function translateMessage(message: Message | string | null): string {
  if (!message) return ''
  return typeof message === 'string' ? t(message) : t(message.key, message.params)
}
