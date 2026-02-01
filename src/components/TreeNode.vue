<script setup lang="ts">
import { Music, Video, FolderOpen as FolderIcon, ChevronRight, ChevronDown } from 'lucide-vue-next'

interface Props {
  item: any
  level: number
  expandedFolders: Set<string>
  currentIndex: number | null
  playlist: any[]
}

const props = defineProps<Props>()
const emit = defineEmits<{
  'toggle-folder': [path: string]
  'play-track': [track: any]
}>()

function isVideo(track: any) {
  if (track.source.Remote) {
    return track.source.Remote.media_type === 'Video'
  }
  if (track.source.Local) {
    const ext = track.source.Local.split('.').pop()?.toLowerCase()
    return ['mp4', 'mkv', 'webm', 'avi', 'mov'].includes(ext || '')
  }
  return false
}

function getFileName(path: string) {
  return path.split(/[/\\]/).pop() || path
}
</script>

<template>
  <div>
    <!-- Folder -->
    <div v-if="item.Folder">
      <div
        @click="emit('toggle-folder', item.Folder.path)"
        class="flex items-center px-2 py-1 rounded cursor-pointer hover:bg-zinc-200 dark:hover:bg-zinc-800 transition-colors select-none"
        :style="{ paddingLeft: (level * 12 + 8) + 'px' }"
      >
        <ChevronDown v-if="expandedFolders.has(item.Folder.path)" class="w-3 h-3 mr-1 text-zinc-400" />
        <ChevronRight v-else class="w-3 h-3 mr-1 text-zinc-400" />
        <FolderIcon class="w-3.5 h-3.5 mr-2 text-zinc-400" />
        <span class="text-xs font-medium">{{ item.Folder.name }}</span>
      </div>

      <div v-if="expandedFolders.has(item.Folder.path)">
        <TreeNode
          v-for="(child, idx) in item.Folder.children"
          :key="idx"
          :item="child"
          :level="level + 1"
          :expanded-folders="expandedFolders"
          :current-index="currentIndex"
          :playlist="playlist"
          @toggle-folder="emit('toggle-folder', $event)"
          @play-track="emit('play-track', $event)"
        />
      </div>
    </div>

    <!-- Track -->
    <div v-else-if="item.Track">
      <div
        @dblclick="emit('play-track', item.Track)"
        class="flex items-center px-2 py-1 rounded cursor-pointer hover:bg-zinc-200 dark:hover:bg-zinc-800 transition-colors select-none"
        :style="{ paddingLeft: (level * 12 + 8) + 'px' }"
      >
        <div class="w-3 h-3 mr-1"></div>
        <Video v-if="isVideo(item.Track)" class="w-3.5 h-3.5 mr-2 text-zinc-400" />
        <Music v-else class="w-3.5 h-3.5 mr-2 text-zinc-400" />
        <span class="text-xs truncate">{{ getFileName(item.Track.source.Local) }}</span>
      </div>
    </div>
  </div>
</template>
