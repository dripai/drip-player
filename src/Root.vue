<script setup lang="ts">
import { onMounted, onUnmounted, ref, watchEffect } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useSettingsStore } from './store/settings'
import { useI18n } from 'vue-i18n'
import App from './App.vue'
import SettingsWindow from './components/SettingsWindow.vue'

const isSettings = getCurrentWindow().label === 'settings'
const { locale, t } = useI18n()
const settings = useSettingsStore()
const error = ref('')
let unlistenError: UnlistenFn | undefined
let disposed = false

watchEffect(() => {
  locale.value = settings.data.language
  document.documentElement.classList.toggle('dark', settings.isDark)
})

function reloadSettings() { window.location.reload() }

async function showContextMenu(event: MouseEvent) {
  // Playlist menus handle their own contextmenu event before it bubbles here.
  if (event.defaultPrevented) return
  event.preventDefault()
  try {
    await invoke('show_app_context_menu', { locale: locale.value })
  } catch (cause) {
    error.value = String(cause)
  }
}

onMounted(async () => {
  document.addEventListener('contextmenu', showContextMenu)
  await settings.initialize()
  try {
    const unlisten = await listen<string>('app-error', event => { error.value = event.payload })
    if (disposed) unlisten()
    else unlistenError = unlisten
  } catch (cause) {
    error.value = String(cause)
  }
})

onUnmounted(() => {
  disposed = true
  document.removeEventListener('contextmenu', showContextMenu)
  unlistenError?.()
  settings.dispose()
})
</script>

<template>
  <template v-if="settings.ready">
    <SettingsWindow v-if="isSettings" />
    <App v-else />
  </template>
  <div v-else class="flex h-screen items-center justify-center bg-zinc-50 text-sm text-zinc-500 dark:bg-zinc-900 dark:text-zinc-400" data-tauri-drag-region>
    {{ settings.error ? t('settings.loadFailed') : t('settings.loading') }}
  </div>
  <div v-if="settings.error" role="alert" class="fixed top-12 left-4 right-4 z-50 rounded-lg border border-red-300 bg-red-50 p-3 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200">
    <p class="break-words">{{ settings.error }}</p>
    <button class="mt-2 underline" @click="reloadSettings">{{ t('settings.reload') }}</button>
  </div>
  <div v-if="error" role="alert" class="fixed bottom-4 left-4 right-4 z-50 flex items-start justify-between gap-4 rounded-lg border border-red-300 bg-red-50 p-3 text-sm text-red-800 shadow-lg dark:border-red-900 dark:bg-red-950 dark:text-red-200">
    <span class="break-words min-w-0">{{ error }}</span>
    <button class="shrink-0 underline" @click="error = ''">{{ t('settings.dismiss') }}</button>
  </div>
</template>
