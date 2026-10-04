<script setup lang="ts">
// Decorative by default: inside a button that already reads "Scanning…" a second
// announcement adds nothing. Pass a label where the spinner is the only signal.
defineProps<{ label?: string; size?: 'small' | 'large' }>()
</script>
<template><span class="spinner" :class="{ 'spinner-large': size === 'large' }" :role="label ? 'status' : undefined" :aria-label="label" :aria-hidden="label ? undefined : 'true'"></span></template>
