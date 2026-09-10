import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { usePreferredDark } from '@vueuse/core'

export type Theme = 'auto' | 'light' | 'dark'
export type Language = 'zh' | 'en'
export type PlayMode = 'sequential' | 'random' | 'repeat_one' | 'repeat_all'

export interface AppSettings {
  revision: number
  theme: Theme
  language: Language
  play_mode: PlayMode
  minimize_to_tray: boolean
}

export type SettingsPatch = Partial<Omit<AppSettings, 'revision'>>

export const useSettingsStore = defineStore('settings', () => {
  // Bootstrap display values only; controls mount after SQLite has been read.
  const data = ref<AppSettings>({
    revision: -1, theme: 'auto', language: 'zh', play_mode: 'sequential', minimize_to_tray: false,
  })
  const ready = ref(false)
  const saving = ref(false)
  const error = ref('')
  const preferredDark = usePreferredDark()
  const isDark = computed(() => data.value.theme === 'dark'
    || (data.value.theme === 'auto' && preferredDark.value))
  let unlisten: UnlistenFn | undefined
  let disposed = false

  function accept(snapshot: AppSettings) {
    if (snapshot.revision > data.value.revision) data.value = snapshot
  }

  async function initialize() {
    try {
      const stop = await listen<AppSettings>('settings-changed', event => accept(event.payload))
      if (disposed) {
        stop()
        return
      }
      unlisten = stop
      const snapshot = await invoke<AppSettings>('get_app_settings')
      if (disposed) return
      accept(snapshot)
      ready.value = true
    } catch (cause) {
      error.value = String(cause)
    }
  }

  async function update(patch: SettingsPatch) {
    if (!ready.value || saving.value) return false
    saving.value = true
    error.value = ''
    try {
      accept(await invoke<AppSettings>('update_app_settings', { patch }))
      return true
    } catch (cause) {
      error.value = String(cause)
      return false
    } finally {
      saving.value = false
    }
  }

  function dispose() {
    disposed = true
    unlisten?.()
  }

  return { data, ready, saving, error, isDark, initialize, update, dispose }
})
