import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type DownloadAuth = 'public' | 'chrome' | 'edge' | 'firefox'
export type DownloadPhase = 'resolving' | 'awaiting_selection' | 'downloading' | 'verifying' | 'publishing' | 'completed' | 'failed' | 'canceled' | 'interrupted' | 'recovery_required'
export interface DownloadOption {
  id: string
  media_type: 'Video' | 'Audio'
  height: number | null
  video_codec: string | null
  extract_audio: boolean
  limited_duration: number | null
}
export interface DownloadJob {
  id: string
  url: string
  title: string
  media_id: string | null
  attempt: number
  phase: DownloadPhase
  directory: string | null
  output_path: string | null
  error: string | null
  interrupt_reason: string | null
  created_at: number
  auth: DownloadAuth
  options: DownloadOption[]
  selection: string | null
  expected_duration: number | null
  progress: { percent: number | null; speed: number | null; eta: number | null } | null
}
interface DownloadSnapshot { revision: number; jobs: DownloadJob[] }
export function isActiveDownload(job: DownloadJob) { return ['resolving', 'downloading', 'verifying', 'publishing'].includes(job.phase) }

export const useDownloadsStore = defineStore('downloads', () => {
  const jobs = ref<DownloadJob[]>([])
  const error = ref('')
  const ready = ref(false)
  const activeCount = computed(() => jobs.value.filter(isActiveDownload).length)
  const clearableCount = computed(() => jobs.value.length - activeCount.value)
  const clearingHistory = ref(false)
  let revision = 0
  let generation = 0
  let unlisten: UnlistenFn | undefined

  async function refresh() {
    const current = generation
    try {
      const snapshot = await invoke<DownloadSnapshot>('get_downloads')
      if (current !== generation || snapshot.revision < revision) return
      revision = snapshot.revision
      jobs.value = snapshot.jobs
      error.value = ''
      ready.value = true
    } catch (cause) { if (current === generation) error.value = String(cause) }
  }

  async function initialize() {
    const current = ++generation
    try {
      const stop = await listen('downloads-updated', () => { void refresh() })
      if (current !== generation) { stop(); return }
      unlisten = stop
      await refresh()
    } catch (cause) { if (current === generation) error.value = String(cause) }
  }

  function dispose() { generation++; unlisten?.(); unlisten = undefined }
  async function submit(url: string, auth: DownloadAuth) {
    const id = await invoke<string>('submit_download', { url, auth })
    await refresh()
    return id
  }
  async function retry(jobId: string) { await invoke('retry_download', { jobId }); await refresh() }
  async function cancel(jobId: string) { await invoke('cancel_download', { jobId }); await refresh() }
  async function select(jobId: string, optionId: string, attempt: number) { await invoke('select_download_format', { jobId, optionId, attempt }); await refresh() }
  async function reparse(jobId: string, auth: DownloadAuth) { await invoke('reparse_download', { jobId, auth }); await refresh() }
  async function clearHistory() {
    if (clearingHistory.value) return
    clearingHistory.value = true
    try { await invoke<number>('clear_download_history'); await refresh() }
    finally { clearingHistory.value = false }
  }
  return { jobs, error, ready, activeCount, clearableCount, clearingHistory, initialize, dispose, refresh, submit, retry, cancel, select, reparse, clearHistory }
})
