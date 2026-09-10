<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Check, CircleAlert, Download, ExternalLink, Folder, ListX, Loader2, Pause, X } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import { isActiveDownload, useDownloadsStore, type DownloadAuth, type DownloadJob, type DownloadOption } from '../store/downloads'
import { useSettingsStore } from '../store/settings'

const { t } = useI18n()
const downloads = useDownloadsStore()
const settings = useSettingsStore()
const url = ref('')
const auth = ref<DownloadAuth>('public')
const accounts: DownloadAuth[] = ['public', 'chrome', 'edge', 'firefox']
const selectedOptions = ref<Record<string, string>>({})
const selectedAccounts = ref<Record<string, DownloadAuth>>({})
const submitting = ref(false)
const formError = ref('')
const rowErrors = ref<Record<string, string>>({})
const busy = ref(new Set<string>())
const list = ref<HTMLElement>()

watch(() => downloads.jobs.map(job => job.id), ids => {
  const remaining = new Set(ids)
  for (const values of [selectedOptions.value, selectedAccounts.value, rowErrors.value]) {
    for (const id of Object.keys(values)) if (!remaining.has(id)) delete values[id]
  }
})

async function submit() {
  if (submitting.value || !url.value.trim()) return
  submitting.value = true
  formError.value = ''
  const submittedUrl = url.value.trim()
  try {
    const id = await downloads.submit(submittedUrl, auth.value)
    if (url.value.trim() === submittedUrl) url.value = ''
    await nextTick()
    list.value?.querySelector<HTMLElement>(`[data-download-id="${id}"]`)?.scrollIntoView({ block: 'nearest' })
  } catch (cause) { formError.value = String(cause) }
  finally { submitting.value = false }
}

async function clearHistory() {
  formError.value = ''
  try { await downloads.clearHistory() }
  catch (cause) { formError.value = String(cause) }
}

async function act(job: DownloadJob, action: 'retry' | 'cancel' | 'login' | 'select' | 'reparse' | 'redownload') {
  if (busy.value.has(job.id) || downloads.clearingHistory) return
  busy.value.add(job.id)
  delete rowErrors.value[job.id]
  try {
    if (action === 'login') await invoke('open_platform_login', { platform: loginPlatform(job) })
    else if (action === 'select') {
      if (account(job) !== job.auth) throw new Error(t('downloads.reparseSource'))
      await downloads.select(job.id, selection(job), job.attempt)
    }
    else if (action === 'reparse' || (action === 'retry' && account(job) !== job.auth)) await downloads.reparse(job.id, account(job))
    else if (action === 'redownload') {
      const id = await downloads.submit(job.url, job.auth)
      await nextTick()
      list.value?.querySelector<HTMLElement>(`[data-download-id="${id}"]`)?.scrollIntoView({ block: 'nearest' })
    }
    else await downloads[action](job.id)
  } catch (cause) { rowErrors.value[job.id] = String(cause) }
  finally { busy.value.delete(job.id) }
}

function account(job: DownloadJob) { return selectedAccounts.value[job.id] ?? job.auth }
function selection(job: DownloadJob) {
  const id = selectedOptions.value[job.id] ?? job.selection
  return job.options.find(option => option.id === id)?.id ?? job.options.find(option => !option.extract_audio && option.limited_duration == null)?.id ?? ''
}
function optionLabel(option: DownloadOption) {
  let label = option.media_type === 'Video'
    ? option.height ? `${option.height}p` : t('downloads.originalVideo')
    : t(option.extract_audio ? 'downloads.mp3Only' : 'downloads.originalAudio')
  if (option.media_type === 'Video' && option.video_codec) label += ` · ${option.video_codec}`
  return option.limited_duration == null ? label : `${label} · ${t('downloads.partial', { duration: duration(option.limited_duration) })}`
}
function accountLabel(value: DownloadAuth) { return value === 'public' ? t('downloads.public') : value === 'chrome' ? 'Chrome' : value === 'edge' ? 'Edge' : 'Firefox' }
function canEdit(job: DownloadJob) { return !isActiveDownload(job) && job.phase !== 'completed' && job.phase !== 'recovery_required' }
function duration(seconds: number) {
  const value = Math.round(seconds)
  const minutes = Math.floor(value / 60)
  return value >= 3600 ? `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}:${String(value % 60).padStart(2, '0')}` : `${minutes}:${String(value % 60).padStart(2, '0')}`
}

function loginPlatform(job: DownloadJob) { return job.error?.match(/^LOGIN_REQUIRED:([^:]+):/)?.[1] }
function errorText(job: DownloadJob) {
  if (rowErrors.value[job.id]) return rowErrors.value[job.id]
  return loginPlatform(job) ? t('downloads.loginNeeded', { platform: loginPlatform(job) }) : job.error
}
function status(job: DownloadJob) {
  if (job.phase === 'interrupted' && job.interrupt_reason === 'directory_changed') return t('downloads.directoryChanged')
  return t(`downloads.phase.${job.phase}`)
}
function speed(bytes: number) {
  const unit = bytes >= 1024 * 1024 ? 'MB/s' : 'KB/s'
  return `${(bytes / (unit === 'MB/s' ? 1024 * 1024 : 1024)).toFixed(1)} ${unit}`
}
async function close() {
  try { await getCurrentWindow().close() } catch (cause) { formError.value = String(cause) }
}
</script>

<template>
  <div class="flex h-screen flex-col overflow-hidden border border-zinc-200 bg-zinc-50 text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100">
    <header data-tauri-drag-region class="flex h-11 shrink-0 select-none items-center justify-between border-b border-zinc-200 pl-5 dark:border-zinc-800">
      <span class="pointer-events-none text-sm font-medium">{{ t('downloads.title') }}</span>
      <button type="button" class="flex h-full w-12 items-center justify-center text-zinc-500 hover:bg-red-500 hover:text-white dark:text-zinc-400 dark:hover:text-white" :aria-label="t('settings.dismiss')" @click="close"><X class="h-4 w-4" /></button>
    </header>
    <form class="shrink-0 px-5 py-4" @submit.prevent="submit">
      <div class="flex items-center gap-3">
        <label for="download-url" class="shrink-0 text-sm">URL</label>
        <input id="download-url" v-model="url" type="url" required autocomplete="off" spellcheck="false" :placeholder="t('downloads.placeholder')" class="min-w-0 flex-1 rounded-md border border-zinc-300 bg-white px-3 py-2 text-sm outline-none focus:border-blue-500 dark:border-zinc-700 dark:bg-zinc-800" />
        <label for="download-auth" class="shrink-0 text-xs">{{ t('downloads.account') }}</label>
        <select id="download-auth" v-model="auth" :disabled="submitting" class="h-9 rounded-md border border-zinc-300 bg-white px-2 text-xs outline-none focus:border-blue-500 dark:border-zinc-700 dark:bg-zinc-800">
          <option v-for="value in accounts" :key="value" :value="value">{{ accountLabel(value) }}</option>
        </select>
        <button type="submit" :disabled="submitting || !url.trim()" class="flex shrink-0 items-center gap-2 rounded-md bg-blue-600 px-4 py-2 text-sm text-white hover:bg-blue-500 disabled:opacity-50"><Loader2 v-if="submitting" class="h-4 w-4 animate-spin" />{{ t('downloads.parse') }}</button>
      </div>
      <p v-if="formError" role="alert" class="mt-2 break-words text-xs text-red-600 dark:text-red-400">{{ formError }}</p>
    </form>
    <div v-if="downloads.error" role="alert" class="mx-5 mb-3 flex items-center gap-3 text-sm text-red-600 dark:text-red-400">
      <span class="min-w-0 flex-1 break-words">{{ downloads.error }}</span><button class="shrink-0 underline" @click="downloads.refresh()">{{ t('downloads.retry') }}</button>
    </div>
    <div ref="list" class="min-h-0 flex-1 overflow-y-auto border-t border-zinc-200 px-5 dark:border-zinc-800">
      <p v-if="downloads.ready && !downloads.jobs.length" class="py-12 text-center text-sm text-zinc-400">{{ t('downloads.empty') }}</p>
      <article v-for="job in downloads.jobs" :key="job.id" :data-download-id="job.id" class="flex gap-3 border-b border-zinc-200 py-4 last:border-0 dark:border-zinc-800">
        <div class="pt-0.5">
          <Loader2 v-if="isActiveDownload(job)" class="h-4 w-4 animate-spin text-blue-500" />
          <Check v-else-if="job.phase === 'completed'" class="h-4 w-4 text-emerald-600 dark:text-emerald-400" />
          <CircleAlert v-else-if="job.phase === 'failed' || job.phase === 'recovery_required'" class="h-4 w-4 text-red-500" />
          <Pause v-else class="h-4 w-4 text-zinc-400" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex items-start justify-between gap-4">
            <div class="min-w-0 flex-1">
              <div class="truncate text-sm font-medium" :title="job.title">{{ job.title }}</div>
              <div v-if="job.title !== job.url" class="mt-1 truncate text-xs text-zinc-400" :title="job.url">{{ job.url }}</div>
            </div>
            <div class="flex shrink-0 items-center gap-3 text-xs">
              <button v-if="loginPlatform(job)" :disabled="busy.has(job.id)" class="flex items-center gap-1 text-blue-600 disabled:opacity-50 dark:text-blue-400" @click="act(job, 'login')"><ExternalLink class="h-3.5 w-3.5" />{{ t('downloads.login') }}</button>
              <button v-if="isActiveDownload(job) || job.phase === 'awaiting_selection'" :disabled="busy.has(job.id)" class="text-zinc-500 hover:text-red-500 disabled:opacity-50" @click="act(job, 'cancel')">{{ t('downloads.cancel') }}</button>
              <button v-else-if="job.phase === 'completed'" :disabled="busy.has(job.id)" class="text-blue-600 disabled:opacity-50 dark:text-blue-400" @click="act(job, 'redownload')">{{ t('downloads.redownload') }}</button>
              <button v-else :disabled="busy.has(job.id)" class="text-blue-600 disabled:opacity-50 dark:text-blue-400" @click="act(job, 'retry')">{{ t(job.phase === 'interrupted' || job.phase === 'canceled' ? 'downloads.continue' : 'downloads.retry') }}</button>
            </div>
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-zinc-500 dark:text-zinc-400">
            <span>{{ status(job) }}</span>
            <span v-if="job.selection && job.options.find(option => option.id === job.selection)">{{ optionLabel(job.options.find(option => option.id === job.selection)!) }}</span>
            <span v-if="job.expected_duration != null" class="tabular-nums">{{ duration(job.expected_duration) }}</span>
            <span v-if="job.phase === 'downloading' && job.progress?.percent != null" class="tabular-nums">{{ job.progress.percent.toFixed(1) }}%</span>
            <span v-if="job.phase === 'downloading' && job.progress?.speed != null" class="tabular-nums">{{ speed(job.progress.speed) }}</span>
            <span v-if="job.phase === 'downloading' && job.progress?.eta != null">{{ t('downloads.eta', { seconds: Math.ceil(job.progress.eta) }) }}</span>
          </div>
          <div v-if="canEdit(job)" class="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-xs">
            <div class="flex items-center gap-2">
            <label :for="`source-${job.id}`">{{ t('downloads.account') }}</label>
            <select :id="`source-${job.id}`" :value="account(job)" :disabled="busy.has(job.id)" class="rounded-md border border-zinc-300 bg-white px-2 py-1.5 outline-none focus:border-blue-500 dark:border-zinc-700 dark:bg-zinc-800" @change="selectedAccounts[job.id] = ($event.target as HTMLSelectElement).value as DownloadAuth">
              <option v-for="value in accounts" :key="value" :value="value">{{ accountLabel(value) }}</option>
            </select>
            <button :disabled="busy.has(job.id)" class="text-blue-600 disabled:opacity-50 dark:text-blue-400" @click="act(job, 'reparse')">{{ t('downloads.reparse') }}</button>
            </div>
            <div v-if="job.phase === 'awaiting_selection'" class="flex min-w-0 items-center gap-2">
            <label :for="`format-${job.id}`">{{ t('downloads.quality') }}</label>
            <select :id="`format-${job.id}`" :value="selection(job)" :disabled="busy.has(job.id) || account(job) !== job.auth" class="min-w-0 max-w-full rounded-md border border-zinc-300 bg-white px-2 py-1.5 outline-none focus:border-blue-500 dark:border-zinc-700 dark:bg-zinc-800" @change="selectedOptions[job.id] = ($event.target as HTMLSelectElement).value">
              <option v-if="!selection(job)" value="" disabled>{{ t(job.options.some(option => option.limited_duration == null) ? 'downloads.selectFormat' : 'downloads.noFullFormat') }}</option>
              <option v-for="option in job.options" :key="option.id" :value="option.id" :disabled="option.limited_duration != null">{{ optionLabel(option) }}</option>
            </select>
            <button :disabled="busy.has(job.id) || !selection(job) || account(job) !== job.auth" class="flex shrink-0 items-center gap-1.5 rounded-md bg-blue-600 px-3 py-1.5 text-white hover:bg-blue-500 disabled:opacity-50" @click="act(job, 'select')"><Download class="h-3.5 w-3.5" />{{ t('downloads.start') }}</button>
            </div>
          </div>
          <progress v-if="job.phase === 'downloading'" :value="job.progress?.percent ?? undefined" max="100" :aria-label="t('downloads.progress')" class="mt-2 block h-1 w-full overflow-hidden rounded-full accent-blue-500" />
          <p v-if="errorText(job)" role="alert" class="mt-2 max-h-24 overflow-y-auto whitespace-pre-wrap break-words text-xs text-red-600 dark:text-red-400">{{ errorText(job) }}</p>
          <p v-if="job.phase === 'completed' && job.output_path" class="mt-1 truncate text-xs text-zinc-400" :title="job.output_path">{{ job.output_path }}</p>
        </div>
      </article>
    </div>
    <footer class="flex shrink-0 items-center gap-2 border-t border-zinc-200 px-5 py-3 text-xs text-zinc-500 dark:border-zinc-800 dark:text-zinc-400">
      <Folder class="h-3.5 w-3.5 shrink-0" /><span class="shrink-0">{{ t('downloads.saveTo') }}</span><span class="min-w-0 flex-1 truncate" :title="settings.data.download_directory">{{ settings.data.download_directory }}</span>
      <button type="button" :disabled="!downloads.ready || !downloads.clearableCount || downloads.clearingHistory || submitting || busy.size > 0" :aria-busy="downloads.clearingHistory"
        class="ml-auto flex shrink-0 items-center gap-1.5 rounded px-2 py-1 text-zinc-600 hover:bg-zinc-200 disabled:opacity-40 dark:text-zinc-300 dark:hover:bg-zinc-800" @click="clearHistory">
        <Loader2 v-if="downloads.clearingHistory" class="h-3.5 w-3.5 animate-spin" /><ListX v-else class="h-3.5 w-3.5" />{{ t('downloads.clearHistory') }}
      </button>
    </footer>
  </div>
</template>

<style scoped>
progress { appearance: none; }
progress::-webkit-progress-bar { background: #71717a33; border-radius: 999px; }
progress::-webkit-progress-value { background: #3b82f6; border-radius: 999px; }
progress::-moz-progress-bar { background: #3b82f6; border-radius: 999px; }
</style>
