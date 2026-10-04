<script setup lang="ts">
defineProps<{ name: string }>()
const paths: Record<string, string> = {
  search: 'M17 10a7 7 0 1 0-14 0 7 7 0 0 0 14 0 M15 15l6 6',
  expand: 'M4 9V4h5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5',
  collapse: 'M9 4v5H4 M20 9h-5V4 M15 20v-5h5 M4 15h5v5',
  projects: 'M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z',
  links: 'M10 13a5 5 0 0 0 7 .5l3-3a5 5 0 0 0-7-7l-2 2 M14 11a5 5 0 0 0-7-.5l-3 3a5 5 0 0 0 7 7l2-2',
  runtime: 'M3 4h18v12H3z M8 21h8 M12 16v5 M7 8l3 2-3 2 M13 12h4',
  health: 'M20 12v8H4V4h12 M2 12h5l3-5 4 10 3-5h5',
  settings: 'M19.21 9.02 L19.53 9.98 L22.34 10.18 L22.34 13.82 L19.53 14.02 L19.21 14.98 L19.21 14.98 L18.75 15.90 L20.60 18.02 L18.02 20.60 L15.90 18.75 L14.98 19.21 L14.98 19.21 L14.02 19.53 L13.82 22.34 L10.18 22.34 L9.98 19.53 L9.02 19.21 L9.02 19.21 L8.10 18.75 L5.98 20.60 L3.40 18.02 L5.25 15.90 L4.79 14.98 L4.79 14.98 L4.47 14.02 L1.66 13.82 L1.66 10.18 L4.47 9.98 L4.79 9.02 L4.79 9.02 L5.25 8.10 L3.40 5.98 L5.98 3.40 L8.10 5.25 L9.02 4.79 L9.02 4.79 L9.98 4.47 L10.18 1.66 L13.82 1.66 L14.02 4.47 L14.98 4.79 L14.98 4.79 L15.90 5.25 L18.02 3.40 L20.60 5.98 L18.75 8.10 L19.21 9.02 Z M15.5 12a3.5 3.5 0 1 0-7 0 3.5 3.5 0 0 0 7 0',
  folder: 'M3 7V4h6l2 3h10v13H3z',
  lock: 'M6 10h12v11H6z M8 10V6a4 4 0 0 1 8 0v4 M12 14v3',
  refresh: 'M20 7v5h-5 M4 17v-5h5 M5 8a8 8 0 0 1 13-3l2 3 M19 16a8 8 0 0 1-13 3l-2-3',
  branch: 'M6 7v10 M6 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4 M6 21a2 2 0 1 0 0-4 2 2 0 0 0 0 4 M18 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4 M18 7v2c0 5-12 3-12 8',
  sun: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M12 2v2 M12 20v2 M2 12h2 M20 12h2 M5 5l1.5 1.5 M17.5 17.5L19 19 M5 19l1.5-1.5 M17.5 6.5L19 5',
  moon: 'M20.5 13A9 9 0 0 1 11 3.5 9 9 0 1 0 20.5 13z',
}
</script>
<template><svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path :d="paths[name] ?? paths.projects" /></svg></template>
