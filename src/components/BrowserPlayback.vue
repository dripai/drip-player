<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import videojs from 'video.js'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { usePlayerStore, type PlaybackSession, type PlaybackSnapshot, type PlaybackStatus } from '../store/player'
import { useLearningStore } from '../store/learning'
import { useI18n } from 'vue-i18n'

const props = defineProps<{ session: PlaybackSession }>()
const emit = defineEmits<{ ready: [player: ReturnType<typeof videojs>, sessionId: number] }>()
const store = usePlayerStore()
const learning = useLearningStore()
const { t } = useI18n()
// The parent keys this component by session ID. Every callback retains its original owner.
const sessionId = props.session.id
const owner = crypto.randomUUID()
const path = props.session.plan?.path || ''
const container = ref<HTMLDivElement | null>(null)
let disposed = false
let player: ReturnType<typeof videojs> | null = null
let sequence = 0
let resumePosition = props.session.position
let autoplay = false

async function report(status?: PlaybackStatus) {
  if (disposed || !player || store.session?.id !== sessionId) return
  const duration = Number(player.duration())
  const position = Number(player.currentTime())
  const report = { session_id: sessionId, owner, sequence: ++sequence,
    position: Number.isFinite(position) ? position : 0,
    duration: Number.isFinite(duration) ? duration : 0,
    status: status || (player.ended() ? 'ended' : player.paused() ? 'paused' : 'playing') }
  try {
    store.acceptSnapshot(await invoke<PlaybackSnapshot>('report_browser_playback', { report }))
  } catch (error) { store.error = String(error) }
}

async function reportError(message: string) {
  if (disposed || store.session?.id !== sessionId) return
  try { await invoke('report_browser_error', { sessionId, owner, message }); await store.syncState() }
  catch (error) { store.error = String(error) }
}

function videoErrorMessage(code: number | undefined, message?: string) {
  const plan = props.session.plan
  return plan?.engine === 'browser_video' && plan.video_codec === 'hevc' && (code === 3 || code === 4)
    ? t('player.hevcUnsupported') : message || t('player.videoFailed')
}

function bindPlayer(instance: ReturnType<typeof videojs>) {
  instance.on('loadedmetadata', async () => {
    if (disposed || store.session?.id !== sessionId) return
    if (resumePosition > 0) instance.currentTime(resumePosition)
    if (autoplay) {
      try { await instance.play() }
      catch (error) {
        if ((error as Error).name === 'NotAllowedError') void report('paused')
        else if ((error as Error).name !== 'AbortError') {
          const code = instance.error()?.code ?? ((error as Error).name === 'NotSupportedError' ? 4 : undefined)
          void reportError(videoErrorMessage(code, String(error)))
        }
      }
    } else { void report('paused') }
  })
  instance.on('timeupdate', () => void report())
  instance.on('durationchange', () => void report())
  instance.on('playing', () => void report('playing'))
  instance.on('pause', () => void report())
  instance.on('waiting', () => void report(instance.paused() ? 'paused' : 'buffering'))
  instance.on('seeked', () => void report())
  instance.on('ended', () => void report('ended'))
  instance.on('error', () => {
    const error = instance.error()
    void reportError(videoErrorMessage(error?.code, error?.message))
  })
}

onMounted(async () => {
  try {
    const snapshot = await invoke<PlaybackSnapshot>('attach_browser_player', { sessionId, owner })
    if (disposed || snapshot.session?.id !== sessionId) return
    store.acceptSnapshot(snapshot)
    resumePosition = snapshot.session.position
    autoplay = ['ready', 'playing', 'buffering'].includes(snapshot.session.status)
    if (!container.value || !path) throw new Error('Video source or container is unavailable')
    const element = document.createElement('video-js')
    element.classList.add('video-js', 'vjs-fill')
    container.value.appendChild(element)
    const instance = videojs(element, {
      controls: false, autoplay: false, preload: 'metadata', fill: true,
      bigPlayButton: false, errorDisplay: false, playsinline: true,
    })
    player = instance
    bindPlayer(instance)
    instance.ready(() => {
      if (disposed || store.session?.id !== sessionId) return
      emit('ready', instance, sessionId)
      instance.src(convertFileSrc(path))
    })
  } catch (error) { await reportError(String(error)) }
})
onBeforeUnmount(() => {
  learning.detach(sessionId)
  disposed = true
  player?.dispose()
  player = null
})
</script>

<template>
  <div ref="container" class="w-full h-full" />
</template>
