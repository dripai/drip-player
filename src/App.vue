<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { usePlayerStore } from './store/player'
import Sidebar from './components/Sidebar.vue'
import Player from './components/Player.vue'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { Moon, Sun, PanelRightOpen, PanelRightClose, Minus, Square, X, Languages } from 'lucide-vue-next'
import { useDark, useToggle } from '@vueuse/core'
import { useI18n } from 'vue-i18n'

const appWindow = getCurrentWindow()
const store = usePlayerStore()
const isDark = useDark()
const toggleDark = useToggle(isDark)
const { locale } = useI18n()

const sidebarVisible = ref(true)
const sidebarWidth = ref(320)
const isResizing = ref(false)

function minimize() { appWindow.minimize() }
function toggleMaximize() { appWindow.toggleMaximize() }
function closeApp() { appWindow.close() }

function toggleSidebar() {
  sidebarVisible.value = !sidebarVisible.value
}

function toggleLanguage() {
  locale.value = locale.value === 'zh' ? 'en' : 'zh'
  localStorage.setItem('locale', locale.value)
}

function startResize(e: MouseEvent) {
  isResizing.value = true
  e.preventDefault()
}

function onMouseMove(e: MouseEvent) {
  if (!isResizing.value) return
  const newWidth = window.innerWidth - e.clientX
  if (newWidth >= 200 && newWidth <= 600) {
    sidebarWidth.value = newWidth
  }
}

function stopResize() {
  isResizing.value = false
}

onMounted(() => {
  document.addEventListener('mousemove', onMouseMove)
  document.addEventListener('mouseup', stopResize)
})

onUnmounted(() => {
  document.removeEventListener('mousemove', onMouseMove)
  document.removeEventListener('mouseup', stopResize)
})

let unlistenState: any
let unlistenPlaylist: any
let unlistenTrackEnded: any
let unlistenPlaybackError: any
let unlistenDownloadProgress: any
let lastProgress = 0
let trackEndedHandled = false

const toastMessage = ref('')
const showToast = ref(false)

function displayToast(msg: string) {
    toastMessage.value = msg
    showToast.value = true
    setTimeout(() => {
        showToast.value = false
    }, 3000)
}

async function checkForUpdates() {
  try {
    const update = await check()
    if (!update) return

    const accepted = window.confirm(`发现新版本 ${update.version}，是否立即更新？`)
    if (!accepted) return

    displayToast(`正在下载 ${update.version} 更新...`)
    await update.downloadAndInstall()
    await relaunch()
  } catch (error) {
    console.error('Update check failed:', error)
  }
}

onMounted(async () => {
  await store.loadPlaylist()
  await store.syncState()
  checkForUpdates()

  unlistenState = await listen('player-state-changed', () => {
    store.syncState()
  })

  unlistenPlaylist = await listen('playlist-updated', () => {
    store.loadPlaylist()
  })

  unlistenTrackEnded = await listen('track-ended', () => {
      store.syncState()
  })

  unlistenPlaybackError = await listen('playback-error', (event) => {
      console.error('Playback error:', event.payload)
      displayToast(event.payload as string)
  })

  unlistenDownloadProgress = await listen('download-progress', (event) => {
      console.log('Download progress:', event.payload)
      displayToast(event.payload as string)
  })

  // Poll progress and handle track end for audio
  setInterval(() => {
    if (store.isPlaying) {
        store.syncState()

        // Check if audio track ended (progress >= 0.99 and was playing)
        if (store.progress >= 0.99 && lastProgress < 0.99 && !trackEndedHandled && store.duration > 0) {
            console.log('Audio track ended, progress:', store.progress, 'play mode:', store.playMode)
            trackEndedHandled = true
            const nextIndex = store.getNextIndex()
            if (nextIndex !== null) {
                store.play(nextIndex)
            } else {
                store.isPlaying = false
            }
        }
        lastProgress = store.progress
    }
  }, 500)

  // Reset trackEndedHandled when track changes
  watch(() => store.currentIndex, () => {
    trackEndedHandled = false
    lastProgress = 0
  })
})

onUnmounted(() => {
  if (unlistenState) unlistenState()
  if (unlistenPlaylist) unlistenPlaylist()
  if (unlistenTrackEnded) unlistenTrackEnded()
  if (unlistenPlaybackError) unlistenPlaybackError()
  if (unlistenDownloadProgress) unlistenDownloadProgress()
})
</script>

<template>
  <div class="flex flex-col h-screen bg-white dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100 transition-colors duration-200">
    <!-- Toast -->
    <div v-if="showToast" class="fixed top-16 right-4 z-50 bg-red-500 text-white px-4 py-2 rounded shadow-lg transition-opacity duration-300">
        {{ toastMessage }}
    </div>

    <!-- Header -->
    <header class="flex items-center justify-between px-4 py-3 border-b dark:border-zinc-800 drag-region" data-tauri-drag-region>
      <div class="flex items-center gap-3">
          <img src="/icon.png" class="w-7 h-7 rounded-sm shadow-sm" alt="Logo" />
      </div>
      <div class="flex items-center gap-2">
        <button @click="toggleSidebar" class="no-drag p-2 rounded-full hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors">
          <PanelRightClose v-if="sidebarVisible" class="w-5 h-5" />
          <PanelRightOpen v-else class="w-5 h-5" />
        </button>
        <button @click="toggleDark()" class="no-drag p-2 rounded-full hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors">
          <Moon v-if="!isDark" class="w-5 h-5" />
          <Sun v-else class="w-5 h-5" />
        </button>
        <button @click="toggleLanguage" class="no-drag p-2 rounded-full hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors" :title="locale === 'zh' ? 'Switch to English' : '切换到中文'">
          <Languages class="w-5 h-5" />
        </button>
        <div class="flex items-center ml-2">
          <button @click="minimize" class="no-drag p-2 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition-colors">
            <Minus class="w-4 h-4" />
          </button>
          <button @click="toggleMaximize" class="no-drag p-2 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition-colors">
            <Square class="w-3.5 h-3.5" />
          </button>
          <button @click="closeApp" class="no-drag p-2 hover:bg-red-500 hover:text-white transition-colors">
            <X class="w-4 h-4" />
          </button>
        </div>
      </div>
    </header>

    <div class="flex flex-1 overflow-hidden relative">
        <Player class="flex-1" />

        <!-- Resize Handle -->
        <div
          v-if="sidebarVisible"
          @mousedown="startResize"
          class="w-1 cursor-col-resize hover:bg-blue-500 transition-colors bg-zinc-200 dark:bg-zinc-800 relative z-10"
          :class="{ 'bg-blue-500': isResizing }"
        ></div>

        <!-- Sidebar -->
        <transition name="slide">
          <Sidebar
            v-if="sidebarVisible"
            :style="{ width: sidebarWidth + 'px' }"
            class="border-l dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-900/50"
          />
        </transition>
    </div>
  </div>
</template>

<style>
.drag-region {
    user-select: none;
    -webkit-app-region: drag;
}
button {
    -webkit-app-region: no-drag;
}

.slide-enter-active,
.slide-leave-active {
  transition: transform 0.3s ease;
}

.slide-enter-from {
  transform: translateX(100%);
}

.slide-leave-to {
  transform: translateX(100%);
}
</style>
