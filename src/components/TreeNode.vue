<script setup lang="ts">
import { Music, Video, FolderOpen as FolderIcon, ChevronRight, ChevronDown } from 'lucide-vue-next'
import { computed } from 'vue'
import { isSourceLocal, isSourceRemote, type LibraryItem, type ResolvedTrack } from '../store/player'
import { trackHasVideo } from '../utils/mediaCapabilities'

interface Props {
  item: LibraryItem
  level: number
  expandedFolders: Set<string>
  currentIndex: number | null
  currentTrack?: ResolvedTrack | null
  playlist: ResolvedTrack[]
}

const props = defineProps<Props>()
const emit = defineEmits<{
  'toggle-folder': [path: string]
  'play-track': [track: ResolvedTrack]
}>()

// Type guards for template
const isFolder = computed(() => 'Folder' in props.item)
const isTrack = computed(() => 'Track' in props.item)
const folder = computed(() => isFolder.value ? (props.item as { Folder: any }).Folder : null)
const track = computed(() => isTrack.value ? (props.item as { Track: ResolvedTrack }).Track : null)

function isVideo(trackItem: ResolvedTrack) {
  return trackHasVideo(trackItem)
}

function getFileName(path: string) {
  return path.split(/[/\\]/).pop() || path
}

function getTrackLabel(trackItem: ResolvedTrack) {
  if (isSourceRemote(trackItem.source) && trackItem.source.Remote.cached_path) {
    return getFileName(trackItem.source.Remote.cached_path)
  }
  if (trackItem.title) {
    return trackItem.title
  }
  if (isSourceLocal(trackItem.source)) {
    return getFileName(trackItem.source.Local.path)
  }
  if (isSourceRemote(trackItem.source)) {
    return trackItem.source.Remote.url
  }
  return 'Unknown Track'
}

function isCurrentTrack(trackItem: ResolvedTrack) {
    if (!props.currentTrack) return false;
    
    // Check if it's the same track by path/url
    if (isSourceLocal(trackItem.source) && isSourceLocal(props.currentTrack.source)) {
        // Normalize paths for comparison
        const p1 = trackItem.source.Local.path.replace(/\\/g, '/').toLowerCase();
        const p2 = props.currentTrack.source.Local.path.replace(/\\/g, '/').toLowerCase();
        return p1 === p2;
    }
    
    if (isSourceRemote(trackItem.source) && isSourceRemote(props.currentTrack.source)) {
        return trackItem.source.Remote.url === props.currentTrack.source.Remote.url;
    }
    
    return false;
}

function onPlayTrack() {
  if (track.value) {
    emit('play-track', track.value)
  }
}
</script>

<template>
  <div>
    <!-- Folder -->
    <div v-if="isFolder && folder">
      <div
        @click="emit('toggle-folder', folder.path)"
        class="flex items-center px-2 py-1 rounded cursor-pointer hover:bg-zinc-200 dark:hover:bg-zinc-800 transition-colors select-none"
        :style="{ paddingLeft: (level * 12 + 8) + 'px' }"
      >
        <ChevronDown v-if="expandedFolders.has(folder.path)" class="w-3 h-3 mr-1 text-zinc-400" />
        <ChevronRight v-else class="w-3 h-3 mr-1 text-zinc-400" />
        <FolderIcon class="w-3.5 h-3.5 mr-2 text-zinc-400" />
        <span class="text-xs font-medium">{{ folder.name }}</span>
      </div>

      <div v-if="expandedFolders.has(folder.path)">
        <TreeNode
          v-for="(child, idx) in folder.children"
          :key="idx"
          :item="child"
          :level="level + 1"
          :expanded-folders="expandedFolders"
          :current-index="currentIndex"
          :current-track="currentTrack"
          :playlist="playlist"
          @toggle-folder="emit('toggle-folder', $event)"
          @play-track="emit('play-track', $event)"
        />
      </div>
    </div>

    <!-- Track -->
    <div v-else-if="isTrack && track">
      <div
        @dblclick="onPlayTrack"
        class="flex items-center px-2 py-1 rounded transition-colors select-none"
        :class="{
            'bg-zinc-200 dark:bg-zinc-800 text-blue-600 dark:text-blue-400': isCurrentTrack(track),
            'cursor-pointer hover:bg-zinc-200 dark:hover:bg-zinc-800': !isCurrentTrack(track)
        }"
        :style="{ paddingLeft: (level * 12 + 8) + 'px' }"
      >
        <div class="w-3 h-3 mr-1"></div>
        <div class="mr-2" :class="{'text-blue-500': isCurrentTrack(track), 'text-zinc-400': !isCurrentTrack(track)}">
             <Video v-if="isVideo(track)" class="w-3.5 h-3.5" />
             <Music v-else class="w-3.5 h-3.5" />
        </div>
        <span class="text-xs truncate">{{ getTrackLabel(track) }}</span>
      </div>
    </div>
  </div>
</template>
