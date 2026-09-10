<script setup lang="ts">
import { ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useI18n } from 'vue-i18n'
import { Monitor, Languages, PanelBottomClose, X } from 'lucide-vue-next'
import { useSettingsStore, type Theme, type Language } from '../store/settings'

const { t, locale } = useI18n()
const settings = useSettingsStore()
const error = ref('')

watch(locale, async () => {
  try {
    await getCurrentWindow().setTitle(`${t('settings.title')} · Drip Player`)
  } catch (cause) {
    error.value = String(cause)
  }
}, { immediate: true })

function changeTheme(event: Event) {
  const input = event.target as HTMLSelectElement
  const theme = input.value as Theme
  input.value = settings.data.theme
  void settings.update({ theme })
}

function changeLanguage(event: Event) {
  const input = event.target as HTMLSelectElement
  const language = input.value as Language
  input.value = settings.data.language
  void settings.update({ language })
}

function changeCloseBehavior(event: Event) {
  const input = event.target as HTMLInputElement
  const enabled = input.checked
  input.checked = settings.data.minimize_to_tray
  void settings.update({ minimize_to_tray: enabled })
}

function reloadSettings() { window.location.reload() }

async function closeSettings() {
  try {
    await getCurrentWindow().close()
  } catch (cause) {
    error.value = String(cause)
  }
}
</script>

<template>
  <div class="flex h-screen flex-col overflow-hidden border border-zinc-200 bg-zinc-50 text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100">
    <header data-tauri-drag-region class="flex h-11 shrink-0 select-none items-center justify-between border-b border-zinc-200 pl-5 dark:border-zinc-800">
      <span class="pointer-events-none text-sm font-medium">{{ t('settings.title') }}</span>
      <button type="button" class="flex h-full w-12 items-center justify-center text-zinc-500 transition-colors hover:bg-red-500 hover:text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-blue-500 dark:text-zinc-400 dark:hover:text-white" :title="t('settings.dismiss')" :aria-label="t('settings.dismiss')" @click="closeSettings">
        <X class="h-4 w-4" />
      </button>
    </header>
    <main class="min-h-0 flex-1 overflow-y-auto px-7 py-7">
      <div class="mx-auto max-w-xl">
        <h1 class="text-xl font-semibold">{{ t('settings.general') }}</h1>
        <p class="mt-1 text-sm text-zinc-500 dark:text-zinc-400">{{ t('settings.savedAutomatically') }}</p>

        <div class="mt-6 divide-y divide-zinc-200 overflow-hidden rounded-xl border border-zinc-200 bg-white dark:divide-zinc-700 dark:border-zinc-700 dark:bg-zinc-800/50">
          <div class="flex items-center justify-between gap-5 p-5">
            <label for="settings-theme" class="flex items-center gap-3 text-sm font-medium">
              <Monitor class="h-4 w-4 text-zinc-500" />
              {{ t('settings.theme') }}
            </label>
            <select id="settings-theme" :value="settings.data.theme" :disabled="settings.saving" class="settings-select" @change="changeTheme">
              <option value="auto">{{ t('settings.themeSystem') }}</option>
              <option value="light">{{ t('settings.themeLight') }}</option>
              <option value="dark">{{ t('settings.themeDark') }}</option>
            </select>
          </div>
          <div class="flex items-center justify-between gap-5 p-5">
            <label for="settings-language" class="flex items-center gap-3 text-sm font-medium">
              <Languages class="h-4 w-4 text-zinc-500" />
              {{ t('settings.language') }}
            </label>
            <select id="settings-language" :value="settings.data.language" :disabled="settings.saving" class="settings-select" @change="changeLanguage">
              <option value="zh">简体中文</option>
              <option value="en">English</option>
            </select>
          </div>
          <div class="flex items-center justify-between gap-5 p-5">
            <label for="settings-tray" class="flex items-start gap-3 cursor-pointer">
              <PanelBottomClose class="mt-0.5 h-4 w-4 shrink-0 text-zinc-500" />
              <span>
                <span class="block text-sm font-medium">{{ t('settings.closeToTray') }}</span>
                <span class="mt-1 block text-xs leading-5 text-zinc-500 dark:text-zinc-400">{{ t('settings.closeToTrayHint') }}</span>
              </span>
            </label>
            <input id="settings-tray" type="checkbox" class="h-4 w-4 shrink-0 cursor-pointer accent-blue-600 disabled:opacity-50" :checked="settings.data.minimize_to_tray" :disabled="settings.saving" @change="changeCloseBehavior" />
          </div>
        </div>

        <div v-if="error" role="alert" class="mt-4 rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-200">
          <p class="break-words">{{ error }}</p>
          <button class="mt-2 underline" @click="reloadSettings">{{ t('settings.reload') }}</button>
        </div>
      </div>
    </main>
  </div>
</template>

<style scoped>
.settings-select {
  @apply w-40 rounded-lg border border-zinc-300 bg-white px-3 py-2 text-sm outline-none focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-zinc-600 dark:bg-zinc-800;
}
</style>
