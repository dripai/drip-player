import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type DownloadPhase = 'resolving' | 'downloading' | 'publishing' | 'completed' | 'failed' | 'canceled' | 'interrupted' | 'recovery_required'
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
  progress: { percent: number | null; speed: number | null; eta: number | null } | null
}
interface DownloadSnapshot { revision: number; jobs: DownloadJob[] }
export function isActiveDownload(job: DownloadJob) { return ['resolving', 'downloading', 'publishing'].includes(job.phase) }

export const useDownloadsStore = defineStore('downloads', () => {
  const jobs = ref<DownloadJob[]>([])
  const error = ref('')
  const ready = ref(false)
  const activeCount = computed(() => jobs.value.filter(isActiveDownload).length)
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
  async function submit(url: string) {
    const id = await invoke<string>('submit_download', { url })
    await refresh()
    return id
  }
  async function retry(jobId: string) { await invoke('retry_download', { jobId }); await refresh() }
  async function cancel(jobId: string) { await invoke('cancel_download', { jobId }); await refresh() }
  return { jobs, error, ready, activeCount, initialize, dispose, refresh, submit, retry, cancel }
})
