<script setup lang="ts">
import { ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open } from '@tauri-apps/plugin-dialog'
import { useI18n } from 'vue-i18n'
import { X } from '@lucide/vue'
import { useSettingsStore, type Theme, type Language } from '../store/settings'
import LearningSettingsForm from './LearningSettingsForm.vue'

const { t } = useI18n()
const settings = useSettingsStore()
const error = ref('')
const downloadDirectory = ref(settings.data.download_directory)
watch(() => settings.data.download_directory, path => { downloadDirectory.value = path })
const section = ref<'general' | 'models' | 'subtitles' | 'shadow' | 'recording'>('general')
const sections = [{ id: 'general', label: '通用' }, { id: 'models', label: '模型' }, { id: 'subtitles', label: '字幕' }, { id: 'shadow', label: '影子跟读' }, { id: 'recording', label: '录音' }] as const

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

async function chooseDownloadDirectory() {
  error.value = ''
  try {
    const path = await open({ directory: true, multiple: false, defaultPath: downloadDirectory.value, title: t('settings.downloadDirectory') })
    if (typeof path === 'string') downloadDirectory.value = path
  } catch (cause) { error.value = String(cause) }
}

async function saveDownloadDirectory() {
  if (settings.saving || !downloadDirectory.value || downloadDirectory.value === settings.data.download_directory) return
  error.value = ''
  if (await settings.setDownloadDirectory(downloadDirectory.value)) downloadDirectory.value = settings.data.download_directory
}

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
    <div class="flex min-h-0 flex-1">
    <nav aria-label="设置分类" class="w-32 shrink-0 space-y-1 border-r border-zinc-200 p-3 dark:border-zinc-800">
      <button v-for="item in sections" :key="item.id" type="button" class="w-full rounded-lg px-3 py-2.5 text-left text-sm hover:bg-zinc-100 dark:hover:bg-zinc-800" :class="section === item.id ? 'bg-blue-50 text-blue-600 dark:bg-blue-950 dark:text-blue-300' : ''" :aria-current="section === item.id ? 'page' : undefined" @click="section = item.id">{{ item.id === 'general' ? t('settings.general') : item.label }}</button>
    </nav>
    <main class="min-h-0 min-w-0 flex-1 overflow-y-auto px-6 py-6">
      <LearningSettingsForm v-if="section !== 'general'" :section="section" class="mx-auto max-w-2xl" />
      <div v-else class="mx-auto max-w-2xl">
        <h1 class="text-xl font-semibold">{{ t('settings.general') }}</h1>

        <div class="mt-5 space-y-3">
          <div class="settings-row">
            <label for="settings-theme">{{ t('settings.theme') }}</label>
            <select id="settings-theme" :value="settings.data.theme" :disabled="settings.saving" class="settings-select" @change="changeTheme">
              <option value="auto">{{ t('settings.themeSystem') }}</option>
              <option value="light">{{ t('settings.themeLight') }}</option>
              <option value="dark">{{ t('settings.themeDark') }}</option>
            </select>
          </div>
          <div class="settings-row">
            <label for="settings-language">{{ t('settings.language') }}</label>
            <select id="settings-language" :value="settings.data.language" :disabled="settings.saving" class="settings-select" @change="changeLanguage">
              <option value="zh">简体中文</option>
              <option value="en">English</option>
            </select>
          </div>
          <div class="settings-row">
            <label for="settings-download-directory">{{ t('settings.downloadDirectory') }}</label>
            <div class="flex min-w-0 items-center gap-2">
              <input id="settings-download-directory" v-model.trim="downloadDirectory" :title="downloadDirectory" :disabled="settings.saving" autocomplete="off" class="settings-select flex-1" @keydown.enter.prevent="saveDownloadDirectory" />
              <button type="button" class="settings-button" :disabled="settings.saving" :aria-label="t('settings.chooseDownloadDirectory')" @click="chooseDownloadDirectory">{{ t('settings.choose') }}</button>
              <button type="button" class="settings-button" :disabled="settings.saving || !downloadDirectory || downloadDirectory === settings.data.download_directory" :aria-label="t('settings.saveDownloadDirectory')" @click="saveDownloadDirectory">{{ settings.saving ? t('settings.saving') : t('settings.save') }}</button>
            </div>
          </div>
          <div class="settings-row">
            <label for="settings-tray" class="cursor-pointer">{{ t('settings.closeToTray') }}</label>
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
  </div>
</template>

<style scoped>
@reference "../style.css";
.settings-row {
  @apply grid min-h-9 items-center gap-x-4 text-sm;
  grid-template-columns: 184px minmax(0, 1fr);
}
.settings-select {
  @apply h-9 w-full min-w-0 rounded-md border border-zinc-300 bg-white px-3 text-sm outline-hidden focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-zinc-600 dark:bg-zinc-800;
}
.settings-button {
  @apply h-9 shrink-0 rounded-md border border-zinc-300 px-3 text-sm hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-600 dark:hover:bg-zinc-800;
}
</style>
