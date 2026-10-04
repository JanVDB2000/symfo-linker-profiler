import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { isTauri } from '@tauri-apps/api/core'
import App from './App.vue'
import { useWorkspace } from './stores/workspace'
import './style.css'

createApp(App).use(createPinia()).mount('#app')

// Start browser development with fixtures unless ?empty requests the initial empty state.
if (import.meta.env.DEV && !isTauri() && !new URLSearchParams(location.search).has('empty')) {
  void useWorkspace().scan()
}
