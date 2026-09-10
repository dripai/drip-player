<script setup lang="ts">
import { computed, ref, shallowRef, onUnmounted, watch } from 'vue'
import type videojs from 'video.js'
import { usePlayerStore, type PlayMode, isSourceLocal, isSourceRemote } from '../store/player'
import { Play, Pause, SkipBack, SkipForward, Volume2, VolumeX, Music, Maximize, Gauge, Repeat, Repeat1, Shuffle, ListOrdered, Subtitles, MonitorPlay } from '@lucide/vue'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import BrowserPlayback from './BrowserPlayback.vue'
import { useI18n } from 'vue-i18n'
import { trackHasVideo } from '../utils/mediaCapabilities'
import { useLearningStore } from '../store/learning'

const { t } = useI18n()
const store = usePlayerStore()
const learning = useLearningStore()
const learningCue = computed(() => learning.transcript?.cues[learning.active] || null)
const videoPlayer = shallowRef<ReturnType<typeof videojs> | null>(null)
const volume = ref(100)
const isMuted = ref(false)
const showVolumeSlider = ref(false)
const playbackRate = ref(1.0)
const showSpeedMenu = ref(false)
const speedOptions = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5]
const showPlayModeMenu = ref(false)
const showSubtitleMenu = ref(false)
const availableSubtitles = ref<{lang: string, path: string}[]>([])
const currentSubtitle = ref<string | null>(null)
const isPlanningBrowserVideo = computed(() => store.session?.status === 'preparing')
// Progress bar dragging state
const isDragging = ref(false)
const dragProgress = ref(0)

// Auto-hide controls for video mode
const showControls = ref(true)
const controlsHideTimer = ref<number | null>(null)
const CONTROLS_HIDE_DELAY = 3000 // 3 seconds of inactivity

const currentTitle = computed(() => {
    if (!store.currentTrack) return 'No Track Playing'
    const t = store.currentTrack
    if (t.title) return t.title
    if (isSourceLocal(t.source)) return t.source.Local.path.split(/[/\\]/).pop()
    if (isSourceRemote(t.source)) return t.source.Remote.url
    return 'Unknown'
})

const isVideo = computed(() => trackHasVideo(store.currentTrack))
const usesBrowserPlayer = computed(() => store.session?.plan?.engine === 'browser_video')
const usesExternalPlayer = computed(() => store.session?.plan?.engine === 'external_video')
const canControl = computed(() => !learning.recording && !learning.recordingStarting && !learning.savingRecording && Boolean(store.session?.plan) && !usesExternalPlayer.value && !['preparing', 'failed', 'stopped'].includes(store.session?.status || ''))
// Display progress (use drag progress when dragging)
const displayProgress = computed(() => {
    return isDragging.value ? dragProgress.value : store.progress * 100
})

// Play mode icon component
const playModeIcon = computed(() => {
    switch (store.playMode) {
        case 'sequential': return ListOrdered
        case 'random': return Shuffle
        case 'repeat_one': return Repeat1
        case 'repeat_all': return Repeat
        default: return ListOrdered
    }
})

watch(() => store.session?.id, () => {
    videoPlayer.value = null
    availableSubtitles.value = []
    currentSubtitle.value = null
    isDragging.value = false
})

watch([() => store.session?.id, () => store.session?.plan?.path], async ([sessionId, path], _, onCleanup) => {
    let cancelled = false
    onCleanup(() => { cancelled = true })
    const mediaId = store.session?.media.id
    if (!mediaId || !path || !usesBrowserPlayer.value) return
    try {
        const assets = await invoke<import('../store/player').MediaAsset[]>('get_media_subtitles', { mediaId })
        if (cancelled || store.session?.id !== sessionId) return
        availableSubtitles.value = assets.map(asset => ({ lang: asset.language || 'und', path: asset.path }))
    } catch (error) { if (!cancelled) store.error = String(error) }
}, { immediate: true })

function selectSubtitle(subtitle: {lang: string, path: string} | null) {
    showSubtitleMenu.value = false
    if (!subtitle) {
        currentSubtitle.value = null
        if (videoPlayer.value) {
            // Remove all text tracks
            // Video.js exposes array-like tracks; its generated declarations omit the indexer.
            const tracks = videoPlayer.value.textTracks() as unknown as ArrayLike<{ mode: TextTrackMode }>
            for (let i = 0; i < tracks.length; i++) {
                tracks[i].mode = 'disabled'
            }
        }
        return
    }

    currentSubtitle.value = subtitle.lang
    if (videoPlayer.value && usesBrowserPlayer.value) {
        // Add subtitle track to video player
        const subtitleUrl = convertFileSrc(subtitle.path)

        // Remove existing tracks first
        const existingTracks = videoPlayer.value.textTracks() as unknown as ArrayLike<{ mode: TextTrackMode }>
        for (let i = existingTracks.length - 1; i >= 0; i--) {
            existingTracks[i].mode = 'disabled'
        }

        // Add new track
        const remoteTrack = videoPlayer.value.addRemoteTextTrack({
            kind: 'subtitles', label: subtitle.lang, src: subtitleUrl, default: true
        }, false)
        // The runtime HTMLTrackElement.track getter is also absent from its declarations.
        const trackElement = remoteTrack as unknown as { track: { mode: TextTrackMode } }
        trackElement.track.mode = 'showing'

    }
}

watch(() => learning.enabled, enabled => {
    if (enabled) selectSubtitle(null)
    else { playbackRate.value = 1; videoPlayer.value?.playbackRate(1) }
})

function cyclePlayMode() {
    const modes: PlayMode[] = ['sequential', 'random', 'repeat_one', 'repeat_all']
    const currentIdx = modes.indexOf(store.playMode)
    const nextIdx = (currentIdx + 1) % modes.length
    store.setPlayMode(modes[nextIdx])
}

// Auto-hide controls logic
function resetControlsTimer() {
    // Clear existing timer
    if (controlsHideTimer.value) {
        clearTimeout(controlsHideTimer.value)
        controlsHideTimer.value = null
    }

    // Show controls
    showControls.value = true

    // Only auto-hide in video mode when playing
    if (isVideo.value && store.isPlaying) {
        controlsHideTimer.value = window.setTimeout(() => {
            // Don't hide if any menu is open or dragging
            if (!showVolumeSlider.value && !showSpeedMenu.value &&
                !showPlayModeMenu.value && !showSubtitleMenu.value && !isDragging.value) {
                showControls.value = false
            }
        }, CONTROLS_HIDE_DELAY)
    }
}

function onPlayerMouseMove() {
    if (!isVideo.value) return

    // Always show controls when mouse moves
    resetControlsTimer()
}

function onPlayerMouseLeave() {
    if (!isVideo.value) return

    // Hide controls faster when mouse leaves (if playing)
    if (store.isPlaying && !showVolumeSlider.value && !showSpeedMenu.value &&
        !showPlayModeMenu.value && !showSubtitleMenu.value && !isDragging.value) {
        if (controlsHideTimer.value) {
            clearTimeout(controlsHideTimer.value)
        }
        controlsHideTimer.value = window.setTimeout(() => {
            showControls.value = false
        }, 500)
    }
}

// Watch for play state changes
watch(() => store.isPlaying, (playing) => {
    if (playing && isVideo.value) {
        resetControlsTimer()
    } else {
        // Always show controls when paused
        showControls.value = true
        if (controlsHideTimer.value) {
            clearTimeout(controlsHideTimer.value)
            controlsHideTimer.value = null
        }
    }
})

// Watch for video mode changes
watch(isVideo, (video) => {
    if (!video) {
        // Always show controls in audio mode
        showControls.value = true
        if (controlsHideTimer.value) {
            clearTimeout(controlsHideTimer.value)
            controlsHideTimer.value = null
        }
    }
})

onUnmounted(() => {
    if (controlsHideTimer.value) clearTimeout(controlsHideTimer.value)
})

function onPlayerReady(player: ReturnType<typeof videojs>, sessionId: number) {
    if (store.session?.id !== sessionId) return
    videoPlayer.value = player
    player.volume(volume.value / 100)
    player.muted(isMuted.value)
    player.playbackRate(playbackRate.value)
    learning.attach(player, sessionId)
}

watch(() => store.session?.plan?.engine, (engine) => {
    if (engine === 'audio' && store.session) {
        void invoke('set_volume', { sessionId: store.session.id, volume: isMuted.value ? 0 : volume.value / 100 })
            .catch(error => { store.error = String(error) })
    }
})

function formatTime(sec: number) {
    if (!sec || isNaN(sec)) return '0:00'
    const m = Math.floor(sec / 60)
    const s = Math.floor(sec % 60)
    return `${m}:${s.toString().padStart(2, '0')}`
}

function onSeekStart() {
    learning.stopLoop(); learning.stopPreview()
    isDragging.value = true
    dragProgress.value = store.progress * 100
}

function onSeekMove(e: Event) {
    if (!isDragging.value) return
    const target = e.target as HTMLInputElement
    dragProgress.value = parseFloat(target.value)
}

function onSeekEnd(e: Event) {
    const target = e.target as HTMLInputElement
    const val = parseFloat(target.value)
    console.log('Seek to:', val, '%, duration:', store.duration, 'seconds')

    isDragging.value = false

    if (store.duration === 0) {
        console.warn('Cannot seek: duration is 0')
        return
    }

    const progress = val / 100

    if (usesBrowserPlayer.value && videoPlayer.value) {
        const player = videoPlayer.value
        const duration = player.duration() ?? 0
        const seekTime = duration * (val / 100)
        console.log('Video seek to:', seekTime, 'seconds (duration:', duration, ')')
        player.currentTime(seekTime)
    } else {
        console.log('Audio seek to progress:', progress)
        store.seek(progress)
            .then(() => {
                console.log('Seek successful')
                setTimeout(() => store.syncState(), 100)
            })
            .catch(err => console.error('Seek failed:', err))
    }
}

async function togglePlayPause() {
    if (!canControl.value) return
    learning.stopLoop(); learning.stopPreview()
    try {
        if (store.session?.status === 'ended') { await store.replay(); return }
        if (usesBrowserPlayer.value && videoPlayer.value) {
            if (videoPlayer.value.paused()) await videoPlayer.value.play()
            else videoPlayer.value.pause()
        } else if (store.isPlaying) await store.pause()
        else await store.resume()
    } catch (error) { store.error = String(error) }
}

function playNext() { void store.advance(false).catch(error => { store.error = String(error) }) }
function playPrevious() { void store.advance(true).catch(error => { store.error = String(error) }) }

function onVolumeChange(e: Event) {
    if (!canControl.value) return
    const target = e.target as HTMLInputElement
    const val = parseInt(target.value)
    volume.value = val
    console.log('Volume change:', val, 'normalized:', val / 100)

    isMuted.value = false
    // 当音量为0时，确保完全静音
    const normalizedVolume = val === 0 ? 0 : val / 100

    if (usesBrowserPlayer.value && videoPlayer.value) {
        videoPlayer.value.muted(false)
        videoPlayer.value.volume(normalizedVolume)
        console.log('Video volume set to:', normalizedVolume)
    } else {
        // Audio volume control via backend
        console.log('Calling set_volume:', normalizedVolume)
        invoke('set_volume', { sessionId: store.session?.id, volume: normalizedVolume })
            .then(() => console.log('Volume set successfully'))
            .catch(err => console.error('Failed to set volume:', err))
    }
}

function toggleMute() {
    if (!canControl.value) return
    isMuted.value = !isMuted.value
    console.log('Toggle mute:', isMuted.value)

    if (usesBrowserPlayer.value && videoPlayer.value) {
        videoPlayer.value.muted(isMuted.value)
    } else {
        // Audio mute via volume
        const vol = isMuted.value ? 0 : volume.value / 100
        console.log('Setting volume for mute:', vol)
        invoke('set_volume', { sessionId: store.session?.id, volume: vol })
            .then(() => console.log('Mute toggled successfully'))
            .catch(err => console.error('Failed to toggle mute:', err))
    }
}

function setPlaybackRate(rate: number) {
    if (!usesBrowserPlayer.value) return
    playbackRate.value = rate
    if (learning.enabled) learning.rate = rate
    showSpeedMenu.value = false

    if (usesBrowserPlayer.value && videoPlayer.value) {
        videoPlayer.value.playbackRate(rate)
    }
    // Audio playback rate would need backend support
}

function toggleFullscreen() {
    if (usesBrowserPlayer.value && videoPlayer.value) {
        if (videoPlayer.value.isFullscreen()) {
            videoPlayer.value.exitFullscreen()
        } else {
            videoPlayer.value.requestFullscreen()
        }
    }
}

</script>

<template>
  <div
    class="flex flex-col h-full bg-white dark:bg-zinc-900 relative"
    @mousemove="onPlayerMouseMove"
    @mouseleave="onPlayerMouseLeave"
  >
    <!-- Main Content (Art / Viz / Video) -->
    <div class="flex-1 flex items-center justify-center p-0 text-zinc-300 dark:text-zinc-700 select-none overflow-hidden relative">

        <div v-if="usesBrowserPlayer && store.session" class="w-full h-full flex items-center justify-center bg-black">
            <BrowserPlayback :key="store.session.id" :session="store.session" @ready="onPlayerReady" />
        </div>

        <div v-else-if="isVideo && isPlanningBrowserVideo" class="w-full h-full flex items-center justify-center bg-black text-zinc-300">
            <span class="text-sm">Preparing video...</span>
        </div>

        <div v-else-if="usesExternalPlayer" class="w-full h-full flex flex-col items-center justify-center bg-black text-zinc-300 gap-4">
            <MonitorPlay class="w-24 h-24 opacity-60" />
            <div class="text-center px-6">
                <h2 class="text-xl font-semibold text-white mb-2 truncate max-w-[80vw]">{{ currentTitle }}</h2>
                <p class="text-sm text-zinc-400">{{ t(store.session?.status === 'external' ? 'player.externalControls' : 'player.externalStopped') }}</p>
            </div>
        </div>

        <div v-else class="text-center w-full max-w-2xl p-8">
            <div class="aspect-square max-h-[400px] rounded-2xl bg-zinc-100 dark:bg-zinc-800 flex items-center justify-center mx-auto mb-8 shadow-2xl border border-gray-200 dark:border-zinc-700/50">
                <Music class="w-32 h-32 opacity-20" />
            </div>
            <h2 class="text-2xl font-bold text-zinc-800 dark:text-zinc-200 mb-2 truncate px-4">{{ currentTitle }}</h2>
            <p class="text-zinc-500 font-medium">{{ t('app.title') }}</p>
        </div>
        <div v-if="learning.enabled && learningCue" class="pointer-events-none absolute left-4 right-4 text-center" :class="isVideo && showControls ? 'bottom-36' : 'bottom-6'">
          <div class="inline-block max-w-full rounded-lg bg-black/75 px-4 py-2 text-white" :style="{ fontSize: `${learning.settings?.subtitle_font_size || 18}px` }">
            <p class="whitespace-pre-line">{{ learning.masked ? '•••' : learningCue.text }}</p>
            <p v-if="!learning.masked && learning.bilingual && learningCue.translation && learningCue.translation_language === learning.settings?.translation_language" class="mt-1 whitespace-pre-line text-sm text-zinc-200">{{ learningCue.translation }}</p>
          </div>
        </div>
    </div>

    <!-- Controls Bar -->
    <transition name="controls-slide">
      <div
        v-show="showControls || !isVideo"
        class="controls-bar border-t border-gray-200 dark:border-zinc-800 px-6 flex flex-col justify-center gap-3"
        :class="{
          'absolute bottom-0 left-0 right-0 bg-linear-to-t from-black/90 via-black/70 to-transparent pt-12 pb-4': isVideo,
          'h-24 bg-white dark:bg-zinc-900': !isVideo
        }"
        @mouseenter="showControls = true"
      >
        <!-- Progress -->
        <div class="flex items-center gap-3 text-xs font-medium" :class="isVideo ? 'text-zinc-300' : 'text-zinc-500'">
            <span class="w-12 text-right tabular-nums">{{ formatTime(store.duration * displayProgress / 100) }}</span>
            <div class="relative flex-1 h-1.5 bg-zinc-200 dark:bg-zinc-700 rounded-full group cursor-pointer">
                <!-- Progress bar -->
                <div
                    class="absolute top-0 left-0 h-full bg-blue-600 rounded-full pointer-events-none"
                    :class="{ 'transition-all': !isDragging }"
                    :style="{ width: `${displayProgress}%` }"
                ></div>
                <!-- Progress indicator -->
                <div
                    class="absolute top-1/2 -translate-y-1/2 w-3 h-3 bg-blue-600 rounded-full shadow-md transition-opacity pointer-events-none"
                    :class="{ 'opacity-100': isDragging, 'opacity-0 group-hover:opacity-100': !isDragging }"
                    :style="{ left: `calc(${displayProgress}% - 6px)` }"
                ></div>
                <!-- Input range -->
                <input
                    type="range"
                    min="0"
                    max="100"
                    :value="displayProgress"
                    @mousedown="onSeekStart"
                    :disabled="!canControl"
                    @touchstart="onSeekStart"
                    @input="onSeekMove"
                    @change="onSeekEnd"
                    class="absolute top-0 left-0 w-full h-full opacity-0 cursor-pointer"
                />
            </div>
            <span class="w-12 tabular-nums">{{ formatTime(store.duration) }}</span>
        </div>

        <!-- Buttons -->
        <div class="flex items-center justify-between">
            <!-- Left: Track info -->
            <div class="flex-1 min-w-0 mr-4">
                <div class="truncate text-sm font-medium text-zinc-800 dark:text-zinc-200">
                    {{ currentTitle }}
                </div>
            </div>

            <!-- Center: Playback controls -->
            <div class="flex items-center gap-4">
                <button
                    @click="playPrevious"
                    class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                    :disabled="!store.currentTrack || learning.recording || learning.recordingStarting || learning.savingRecording || !!learning.pendingRecording"
                    title="Previous (Ctrl+Left)"
                >
                    <SkipBack class="w-5 h-5" />
                </button>

                <button
                    @click="togglePlayPause"
                    class="w-12 h-12 rounded-full bg-blue-600 hover:bg-blue-700 text-white flex items-center justify-center shadow-lg transition-all hover:scale-105 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed"
                    :disabled="!canControl"
                    title="Play/Pause (Space)"
                >
                    <Pause v-if="store.isPlaying" class="w-5 h-5 fill-current" />
                    <Play v-else class="w-5 h-5 fill-current ml-0.5" />
                </button>

                <button
                    @click="playNext"
                    class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
                    :disabled="!store.currentTrack || learning.recording || learning.recordingStarting || learning.savingRecording || !!learning.pendingRecording"
                    title="Next (Ctrl+Right)"
                >
                    <SkipForward class="w-5 h-5" />
                </button>
            </div>

            <!-- Right: Volume and controls -->
            <div class="flex-1 flex justify-end items-center gap-3">
                <!-- Play mode -->
                <div class="relative" @mouseenter="showPlayModeMenu = true" @mouseleave="showPlayModeMenu = false">
                    <button
                        @click="cyclePlayMode"
                        class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors"
                        :title="t(`player.playMode.${store.playMode}`)"
                    >
                        <component :is="playModeIcon" class="w-5 h-5" />
                    </button>

                    <!-- Play mode menu -->
                    <transition name="fade">
                        <div
                            v-show="showPlayModeMenu"
                            class="absolute bottom-full right-0 mb-2 bg-white dark:bg-zinc-800 rounded-lg shadow-xl py-1 min-w-[120px]"
                        >
                            <button
                                @click="store.setPlayMode('sequential'); showPlayModeMenu = false"
                                class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors flex items-center gap-2"
                                :class="{ 'text-blue-600 dark:text-blue-400 font-medium': store.playMode === 'sequential' }"
                            >
                                <ListOrdered class="w-4 h-4" />
                                {{ t('player.playMode.sequential') }}
                            </button>
                            <button
                                @click="store.setPlayMode('random'); showPlayModeMenu = false"
                                class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors flex items-center gap-2"
                                :class="{ 'text-blue-600 dark:text-blue-400 font-medium': store.playMode === 'random' }"
                            >
                                <Shuffle class="w-4 h-4" />
                                {{ t('player.playMode.random') }}
                            </button>
                            <button
                                @click="store.setPlayMode('repeat_one'); showPlayModeMenu = false"
                                class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors flex items-center gap-2"
                                :class="{ 'text-blue-600 dark:text-blue-400 font-medium': store.playMode === 'repeat_one' }"
                            >
                                <Repeat1 class="w-4 h-4" />
                                {{ t('player.playMode.repeat_one') }}
                            </button>
                            <button
                                @click="store.setPlayMode('repeat_all'); showPlayModeMenu = false"
                                class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors flex items-center gap-2"
                                :class="{ 'text-blue-600 dark:text-blue-400 font-medium': store.playMode === 'repeat_all' }"
                            >
                                <Repeat class="w-4 h-4" />
                                {{ t('player.playMode.repeat_all') }}
                            </button>
                        </div>
                    </transition>
                </div>

                <!-- Subtitle selector (video only) -->
                <div v-if="usesBrowserPlayer && !learning.enabled" class="relative" @mouseenter="showSubtitleMenu = true" @mouseleave="showSubtitleMenu = false">
                    <button
                        class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors"
                        :class="{ 'text-blue-600 dark:text-blue-400': currentSubtitle }"
                        :title="t('player.subtitle.title')"
                    >
                        <Subtitles class="w-5 h-5" />
                    </button>

                    <!-- Subtitle menu -->
                    <transition name="fade">
                        <div
                            v-show="showSubtitleMenu"
                            class="absolute bottom-full right-0 mb-2 bg-white dark:bg-zinc-800 rounded-lg shadow-xl py-1 min-w-[120px]"
                        >
                            <button
                                @click="selectSubtitle(null)"
                                class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
                                :class="{ 'text-blue-600 dark:text-blue-400 font-medium': !currentSubtitle }"
                            >
                                {{ t('player.subtitle.off') }}
                            </button>
                            <template v-if="availableSubtitles.length > 0">
                                <button
                                    v-for="sub in availableSubtitles"
                                    :key="sub.path"
                                    @click="selectSubtitle(sub)"
                                    class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
                                    :class="{ 'text-blue-600 dark:text-blue-400 font-medium': currentSubtitle === sub.lang }"
                                >
                                    {{ sub.lang }}
                                </button>
                            </template>
                            <div v-else class="px-3 py-1.5 text-xs text-zinc-400">
                                {{ t('player.subtitle.noSubtitles') }}
                            </div>
                        </div>
                    </transition>
                </div>

                <!-- Playback speed -->
                <div class="relative" @mouseenter="showSpeedMenu = usesBrowserPlayer" @mouseleave="showSpeedMenu = false">
                    <button
                        :disabled="!usesBrowserPlayer"
                        class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors text-xs font-medium"
                        title="Playback speed"
                    >
                        <div class="flex items-center gap-1">
                            <Gauge class="w-4 h-4" />
                            <span>{{ usesBrowserPlayer ? (learning.enabled ? learning.rate : playbackRate) : 1 }}x</span>
                        </div>
                    </button>

                    <!-- Speed menu -->
                    <transition name="fade">
                        <div
                            v-show="showSpeedMenu"
                            class="absolute bottom-full right-0 mb-2 bg-white dark:bg-zinc-800 rounded-lg shadow-xl py-1 min-w-[80px]"
                        >
                            <button
                                v-for="speed in speedOptions"
                                :key="speed"
                                @click="setPlaybackRate(speed)"
                                class="w-full px-3 py-1.5 text-xs text-left hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors"
                                :class="{ 'text-blue-600 dark:text-blue-400 font-medium': playbackRate === speed }"
                            >
                                {{ speed }}x
                            </button>
                        </div>
                    </transition>
                </div>

                <!-- Fullscreen (video only) -->
                <button
                    v-if="usesBrowserPlayer"
                    @click="toggleFullscreen"
                    class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors"
                    title="Fullscreen (F)"
                >
                    <Maximize class="w-5 h-5" />
                </button>

                <!-- Volume control -->
                <div class="relative" @mouseenter="showVolumeSlider = canControl" @mouseleave="showVolumeSlider = false">
                    <button
                        @click="toggleMute"
                        :disabled="!canControl"
                        class="text-zinc-600 hover:text-zinc-900 dark:text-zinc-400 dark:hover:text-zinc-100 transition-colors"
                        :title="isMuted ? 'Unmute' : 'Mute'"
                    >
                        <VolumeX v-if="isMuted || volume === 0" class="w-5 h-5" />
                        <Volume2 v-else class="w-5 h-5" />
                    </button>

                    <!-- Volume slider -->
                    <transition name="fade">
                        <div
                            v-show="showVolumeSlider"
                            class="absolute bottom-full right-0 mb-2 bg-white dark:bg-zinc-800 rounded-lg shadow-xl p-2 w-8"
                        >
                            <input
                                type="range"
                                min="0"
                                max="100"
                                :value="volume"
                                :disabled="!canControl"
                                @input="onVolumeChange"
                                class="volume-slider"
                                orient="vertical"
                            />
                        </div>
                    </transition>
                </div>

                <span class="text-xs tabular-nums w-8 text-right" :class="isVideo ? 'text-zinc-300' : 'text-zinc-500'">{{ volume }}%</span>
            </div>
        </div>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.volume-slider {
    -webkit-appearance: slider-vertical;
    writing-mode: bt-lr;
    width: 100%;
    height: 100px;
    cursor: pointer;
}

.fade-enter-active,
.fade-leave-active {
    transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
    opacity: 0;
}

/* Controls slide animation */
.controls-slide-enter-active,
.controls-slide-leave-active {
    transition: transform 0.3s ease, opacity 0.3s ease;
}

.controls-slide-enter-from,
.controls-slide-leave-to {
    transform: translateY(100%);
    opacity: 0;
}

/* Video mode specific styles */
.controls-bar.absolute {
    z-index: 10;
}

/* Override text colors for video mode (on dark gradient background) */
.controls-bar.absolute .text-zinc-600 {
    color: rgb(212 212 216); /* zinc-300 */
}

.controls-bar.absolute .text-zinc-600:hover {
    color: rgb(255 255 255); /* white */
}

.controls-bar.absolute .dark\:text-zinc-400 {
    color: rgb(212 212 216); /* zinc-300 */
}

.controls-bar.absolute .dark\:text-zinc-400:hover {
    color: rgb(255 255 255); /* white */
}

.controls-bar.absolute .text-zinc-800,
.controls-bar.absolute .dark\:text-zinc-200 {
    color: rgb(255 255 255); /* white */
}

.controls-bar.absolute .text-zinc-500 {
    color: rgb(161 161 170); /* zinc-400 */
}

.controls-bar.absolute .bg-zinc-200,
.controls-bar.absolute .dark\:bg-zinc-700 {
    background-color: rgba(255, 255, 255, 0.3);
}
</style>
