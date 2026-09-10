<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch, watchEffect } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useSettingsStore } from './store/settings'
import { useI18n } from 'vue-i18n'
import App from './App.vue'
import SettingsWindow from './components/SettingsWindow.vue'
import DownloadsWindow from './components/DownloadsWindow.vue'
import { useDownloadsStore } from './store/downloads'
import { RefreshCw, Settings } from '@lucide/vue'
import ContextMenu from './components/ContextMenu.vue'
import { closeContextMenu, openContextMenu } from './composables/contextMenu'

const currentWindow = getCurrentWindow()
const isSettings = currentWindow.label === 'settings'
const isDownloads = currentWindow.label === 'downloads'
const downloads = useDownloadsStore()
const { locale, t } = useI18n()
const settings = useSettingsStore()
const error = ref('')
let unlistenError: UnlistenFn | undefined
let disposed = false

watchEffect(() => {
  locale.value = settings.data.language
  document.documentElement.lang = settings.data.language
  document.documentElement.classList.toggle('dark', settings.isDark)
})

watch(() => [settings.ready, locale.value] as const, ([ready]) => {
  if (!ready) return
  const title = isSettings ? `${t('settings.title')} · ${t('app.title')}` : isDownloads ? `${t('downloads.title')} · ${t('app.title')}` : t('app.title')
  document.title = title
  void currentWindow.setTitle(title).catch(cause => { error.value = String(cause) })
}, { immediate: true })

function reloadSettings() { window.location.reload() }

function showContextMenu(event: MouseEvent) {
  if (event.defaultPrevented) return
  if (event.target instanceof Element && event.target.closest('dialog[open]')) {
    event.preventDefault()
    return
  }
  openContextMenu(event, [
    { id: 'refresh', label: t('menu.refresh'), icon: RefreshCw, action: () => window.location.reload() },
    { id: 'settings', label: t('settings.title'), icon: Settings, action: async () => { await invoke('open_settings_window', { locale: locale.value }) } },
  ])
}

watch(() => [settings.isDark, locale.value], closeContextMenu)

onMounted(async () => {
  document.addEventListener('contextmenu', showContextMenu)
  await settings.initialize()
  if (!isSettings && !disposed) await downloads.initialize()
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
  downloads.dispose()
})
</script>

<template>
  <ContextMenu @error="error = $event" />
  <template v-if="settings.ready">
    <SettingsWindow v-if="isSettings" />
    <DownloadsWindow v-else-if="isDownloads" />
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
