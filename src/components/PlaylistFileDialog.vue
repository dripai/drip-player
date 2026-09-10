<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from 'vue-i18n'
import { usePlayerStore, type PlaylistItem } from '../store/player'
import { useLearningStore } from '../store/learning'

interface PlaylistFile {
  item_id: string
  media_id: string
  path: string
  stamp: string
  directory_revision: number
  stem: string
  extension: string
}

const props = defineProps<{ item: PlaylistItem; mode: 'rename' | 'delete' }>()
const emit = defineEmits<{ close: [] }>()
const { t } = useI18n()
const player = usePlayerStore()
const learning = useLearningStore()
const dialog = ref<HTMLDialogElement | null>(null)
const input = ref<HTMLInputElement | null>(null)
const cancel = ref<HTMLButtonElement | null>(null)
const file = ref<PlaylistFile | null>(null)
const name = ref('')
const error = ref('')
const busy = ref(false)
let disposed = false

onMounted(async () => {
  dialog.value?.showModal()
  cancel.value?.focus()
  try {
    const target = await invoke<PlaylistFile>('get_playlist_file', { itemId: props.item.id })
    if (disposed) return
    file.value = target
    name.value = target.stem
    if (props.mode === 'rename') {
      await nextTick()
      input.value?.focus()
      input.value?.select()
    }
  } catch (cause) { error.value = String(cause) }
})
onUnmounted(() => { disposed = true; dialog.value?.close() })

function close() { if (!busy.value) emit('close') }
async function submit() {
  if (!file.value || busy.value) return
  if (player.session?.media.id === file.value.media_id && (learning.recording || learning.recordingStarting || learning.savingRecording || learning.pendingRecording)) {
    error.value = t('fileAction.recordingBusy')
    return
  }
  busy.value = true
  error.value = ''
  try {
    await invoke(props.mode === 'rename' ? 'rename_playlist_file' : 'delete_playlist_file', {
      target: file.value, ...(props.mode === 'rename' ? { name: name.value } : {}),
    })
    emit('close')
  } catch (cause) { error.value = String(cause) }
  finally {
    await player.loadPlaylist()
    await player.syncState()
    busy.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <dialog ref="dialog" :aria-label="t(mode === 'rename' ? 'menu.rename' : 'fileAction.deleteTitle')" :aria-busy="busy" @cancel.prevent="close"
      class="m-auto w-[440px] max-w-[calc(100vw-32px)] rounded-xl border border-zinc-200 bg-white p-5 text-zinc-900 shadow-2xl backdrop:bg-black/40 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100">
      <form @submit.prevent="submit">
        <h2 class="mb-5 text-base font-semibold">{{ t(mode === 'rename' ? 'menu.rename' : 'fileAction.deleteTitle') }}</h2>
        <template v-if="mode === 'rename'">
          <label class="flex items-center gap-4 text-sm">
            <span class="shrink-0">{{ t('fileAction.name') }}</span>
            <span class="flex min-w-0 flex-1 items-center rounded-md border border-zinc-300 bg-zinc-50 focus-within:border-blue-500 dark:border-zinc-600 dark:bg-zinc-800">
              <input ref="input" v-model="name" :disabled="!file || busy" required autocomplete="off" spellcheck="false" class="min-w-0 flex-1 bg-transparent px-3 py-2 outline-none" />
              <span v-if="file" class="pr-3 text-zinc-500 dark:text-zinc-400">.{{ file.extension }}</span>
            </span>
          </label>
        </template>
        <template v-else>
          <p class="mb-2 break-words text-sm font-medium">{{ file ? `${file.stem}.${file.extension}` : item.title }}</p>
          <p class="text-sm leading-6 text-zinc-600 dark:text-zinc-400">{{ t('fileAction.deleteHint') }}</p>
          <p v-if="file" class="mt-3 break-all text-xs leading-5 text-zinc-500">{{ file.path }}</p>
        </template>
        <p v-if="error" role="alert" class="mt-4 break-words text-sm text-red-600 dark:text-red-400">{{ error }}</p>
        <div class="mt-6 flex justify-end gap-2">
          <button ref="cancel" type="button" :disabled="busy" class="rounded-md border border-zinc-300 px-4 py-2 text-sm hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-600 dark:hover:bg-zinc-800" @click="close">{{ t('fileAction.cancel') }}</button>
          <button type="submit" :disabled="!file || busy || (mode === 'rename' && (!name.trim() || name === file.stem))" class="rounded-md px-4 py-2 text-sm text-white disabled:opacity-40"
            :class="mode === 'delete' ? 'bg-red-600 hover:bg-red-700' : 'bg-blue-600 hover:bg-blue-700'">
            {{ busy ? t('fileAction.working') : t(mode === 'delete' ? 'fileAction.deleteConfirm' : 'settings.save') }}
          </button>
        </div>
      </form>
    </dialog>
  </Teleport>
</template>
