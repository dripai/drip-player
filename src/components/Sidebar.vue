<script setup lang="ts">
import { usePlayerStore, type PlaylistItem } from '../store/player'
import { ref } from 'vue'
import { Music, Video, RefreshCw, Pencil, Trash2, ListMinus } from '@lucide/vue'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from 'vue-i18n'
import { openContextMenu } from '../composables/contextMenu'
import PlaylistFileDialog from './PlaylistFileDialog.vue'

const store = usePlayerStore()
const { t } = useI18n()
const fileAction = ref<{ item: PlaylistItem; mode: 'rename' | 'delete' } | null>(null)

async function play(item: PlaylistItem) {
  try { await store.play(item.id) }
  catch (cause) { store.error = String(cause) }
}

function showContextMenu(event: MouseEvent, item?: PlaylistItem) {
  const localFile = item && (item.origin.kind === 'local' || !!item.cached_path)
  openContextMenu(event, item ? [
    { id: 'rename', label: t('menu.rename'), icon: Pencil, disabled: !localFile, action: () => { fileAction.value = { item, mode: 'rename' } } },
    { id: 'remove', label: t('menu.removeFromPlaylist'), icon: ListMinus, action: async () => { await invoke('remove_track', { itemId: item.id }); await store.loadPlaylist(); await store.syncState() } },
    { id: 'delete', label: t('menu.delete'), icon: Trash2, danger: true, separatorBefore: true, disabled: !localFile, action: () => { fileAction.value = { item, mode: 'delete' } } },
  ] : [
    { id: 'refresh', label: t('sidebar.refresh'), icon: RefreshCw, action: () => store.refreshPlaylist() },
    { id: 'clear', label: t('menu.clearPlaylist'), icon: ListMinus, disabled: !store.playlist.length, action: async () => { await invoke('clear_playlist'); await store.loadPlaylist(); await store.syncState() } },
  ])
}
</script>

<template>
  <div class="flex h-full flex-col bg-zinc-50 dark:bg-zinc-900/50">
    <div class="flex items-center justify-between border-b border-gray-200 p-4 dark:border-zinc-800" @contextmenu="showContextMenu($event)">
      <div class="flex min-w-0 items-center gap-2">
        <h2 class="text-sm font-semibold uppercase text-zinc-500 dark:text-zinc-400">{{ t('sidebar.playlist') }}</h2>
        <span class="text-xs text-zinc-400">{{ store.playlist.length }}</span>
      </div>
      <button type="button" class="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-200 disabled:opacity-50 dark:text-zinc-400 dark:hover:bg-zinc-800" :title="t('sidebar.refresh')" :aria-label="t('sidebar.refresh')" :aria-busy="store.refreshingPlaylist" :disabled="store.refreshingPlaylist" @click="store.refreshPlaylist()">
        <RefreshCw class="h-4 w-4" :class="{ 'animate-spin': store.refreshingPlaylist }" />
      </button>
    </div>
    <div class="flex-1 space-y-0.5 overflow-y-auto p-2">
      <div v-for="item in store.playlist" :key="item.id" :data-playlist-item-id="item.id" tabindex="0" @keydown.enter="play(item)" @dblclick="play(item)" @contextmenu="showContextMenu($event, item)"
        class="group flex cursor-pointer select-none items-center rounded-sm px-2 py-1 transition-colors hover:bg-zinc-200 dark:hover:bg-zinc-800"
        :class="{ 'bg-zinc-200 text-blue-600 dark:bg-zinc-800 dark:text-blue-400': store.currentItemId === item.id }">
        <div class="mr-2 text-zinc-400" :class="{ 'text-blue-500': store.currentItemId === item.id }">
          <Video v-if="item.media_type === 'Video'" class="h-3.5 w-3.5" />
          <Music v-else class="h-3.5 w-3.5" />
        </div>
        <div class="min-w-0 flex-1 truncate text-xs font-medium" :title="item.title">{{ item.title }}</div>
      </div>
    </div>
    <PlaylistFileDialog v-if="fileAction" :item="fileAction.item" :mode="fileAction.mode" @close="fileAction = null" />
  </div>
</template>
