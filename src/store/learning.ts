import { computed, ref, shallowRef, watch } from 'vue'
import { defineStore } from 'pinia'
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type videojs from 'video.js'
import { usePlayerStore, type MediaAsset, type PlaybackSnapshot } from './player'
import { ShadowLoop, activeCueIndex, cueSegment } from '../learning/shadowLoop'

export interface LearningSettings {
  revision: number; workspace_id: string; text_model: string; transcription_model: string; evaluation_app_id: string
  subtitle_display: 'original' | 'bilingual'; translation_language: string; subtitle_font_size: number; auto_scroll: boolean
  playback_rate: number; repetitions: number; gap_seconds: number; skip_silence: boolean
  recording_device: string; recording_directory: string; playback_order: 'original_first' | 'recording_first'
}
export interface Cue { id: number; start: number; end: number; text: string; translation: string | null; translation_language: string | null; favorite: boolean }
export interface Transcript { id: string; media_id: string; label: string; cues: Cue[] }
export interface Recording { id: string; transcript_id: string; cue_id: number; path: string; created_at: number; evaluation: string | null }

export const useLearningStore = defineStore('learning', () => {
  const playback = usePlayerStore()
  const settings = ref<LearningSettings | null>(null)
  const enabled = ref(false), error = ref(''), busy = ref(false), loading = ref(false)
  const transcripts = ref<Transcript[]>([]), transcript = ref<Transcript | null>(null)
  const selected = ref(0), active = ref(-1), position = ref(0)
  const recordings = ref<Recording[]>([]), selectedRecording = ref<string | null>(null)
  const repeat = ref(false), ab = ref(false), a = ref<number | null>(null), b = ref<number | null>(null)
  const rate = ref(1), repetitions = ref(2), gap = ref(1), skipSilence = ref(false)
  const masked = ref(false), bilingual = ref(true), favoritesOnly = ref(false), offset = ref(0)
  const explanation = ref(''), jobs = ref<string[]>([])
  const recording = ref(false), savingRecording = ref(false), recordingSeconds = ref(0), pendingRecording = shallowRef<Blob | null>(null)
  const recordingStarting = ref(false)
  const recordingTarget = ref<{ transcriptId: string; cueId: number } | null>(null)
  const browser = shallowRef<ReturnType<typeof videojs> | null>(null)
  const browserSession = ref<number | null>(null)
  const canControl = computed(() => Boolean(playback.session?.plan) && playback.session?.plan?.engine !== 'external_video' && !['preparing', 'failed', 'stopped'].includes(playback.session?.status || ''))
  const cue = computed(() => transcript.value?.cues[selected.value] || null)
  const currentRecording = computed(() => recordings.value.find(r => r.id === selectedRecording.value) || null)
  let generation = 0, operation = 0, frame = 0, loop: ShadowLoop | null = null, executing = false
  let unlisten: UnlistenFn | undefined, initialized: Promise<void> | undefined
  let recorder: MediaRecorder | null = null, input: MediaStream | null = null, recordingTimer: ReturnType<typeof setInterval> | undefined
  let preview: HTMLAudioElement | null = null
  let originalEnd: number | null = null, afterOriginal: (() => void) | null = null

  function accept(value: LearningSettings) { if (!settings.value || value.revision > settings.value.revision) settings.value = value }
  async function initialize() {
    if (!initialized) initialized = (async () => {
      try { unlisten = await listen<LearningSettings>('learning-settings-changed', e => accept(e.payload)); accept(await invoke('get_learning_settings')) }
      catch (cause) { error.value = String(cause); initialized = undefined }
    })()
    await initialized
  }
  function defaults() {
    const s = settings.value
    if (!s) return
    rate.value = s.playback_rate; repetitions.value = s.repetitions; gap.value = s.gap_seconds
    skipSilence.value = s.skip_silence; bilingual.value = s.subtitle_display === 'bilingual'
  }
  function time() { return browser.value && browserSession.value === playback.session?.id ? (browser.value.currentTime() || 0) : (playback.session?.position || 0) }
  async function seekTo(seconds: number) {
    const session = playback.session
    if (!session || !canControl.value) throw new Error('当前播放引擎不支持学习控制')
    const value = Math.max(0, Math.min(seconds, session.duration || seconds))
    if (session.plan?.engine === 'browser_video') { if (!browser.value) throw new Error('播放器尚未就绪'); browser.value.currentTime(value) }
    else { await invoke('seek', { sessionId: session.id, position: value }); playback.acceptSnapshot(await invoke<PlaybackSnapshot>('get_state')) }
  }
  async function pause() {
    if (browser.value && browserSession.value === playback.session?.id) browser.value.pause()
    else if (canControl.value) { await playback.pause(); playback.acceptSnapshot(await invoke<PlaybackSnapshot>('get_state')) }
  }
  async function play() {
    if (browser.value && browserSession.value === playback.session?.id) await browser.value.play()
    else if (canControl.value) { await playback.resume(); playback.acceptSnapshot(await invoke<PlaybackSnapshot>('get_state')) }
  }
  function stopLoop() { loop?.stop(); loop = null; repeat.value = false; ab.value = false; operation++; executing = false }
  function stopPreview() {
    preview?.pause(); if (preview) { preview.removeAttribute('src'); preview.load() }; preview = null
    originalEnd = null; afterOriginal = null
  }
  async function jump(index: number, autoPlay = true) {
    stopLoop(); stopPreview()
    const target = transcript.value?.cues[index]
    if (!target || recording.value || recordingStarting.value || savingRecording.value || pendingRecording.value) return
    const segment = cueSegment(target, offset.value, playback.duration)
    selected.value = index; explanation.value = ''; selectedRecording.value = recordings.value.find(r => r.cue_id === target.id)?.id || null
    const token = operation
    await seekTo(segment.start)
    if (autoPlay && token === operation) await play()
  }
  async function toggleRepeat() {
    if (repeat.value) { stopLoop(); return }
    if (!transcript.value || !cue.value) return
    if (!Number.isInteger(repetitions.value) || repetitions.value < 1 || repetitions.value > 10 || !Number.isFinite(gap.value) || gap.value < 0 || gap.value > 10) throw new Error('每句次数为 1–10，停顿为 0–10 秒')
    stopLoop(); stopPreview()
    const segments = transcript.value.cues.slice(selected.value).filter(c => !favoritesOnly.value || c.favorite).map(c => cueSegment(c, offset.value, playback.duration))
    loop = new ShadowLoop(segments, repetitions.value, gap.value)
    repeat.value = true
    const token = operation
    try { await seekTo(segments[0].start); if (token === operation) await play() } catch (cause) { stopLoop(); throw cause }
  }
  function markA() { stopLoop(); a.value = time(); b.value = null }
  async function markB() {
    const end = time()
    if (a.value === null || end - a.value < 0.1) throw new Error('B 点必须在 A 点之后')
    stopLoop(); stopPreview(); b.value = end
    loop = new ShadowLoop([{ start: a.value, end }], 1, gap.value, true); ab.value = true
    const token = operation
    try { await seekTo(a.value); if (token === operation) await play() } catch (cause) { stopLoop(); throw cause }
  }
  function attach(player: ReturnType<typeof videojs>, id: number) {
    browser.value = player; browserSession.value = id
    if (enabled.value) player.playbackRate(rate.value)
  }
  function detach(id: number) { if (browserSession.value === id) { browser.value = null; browserSession.value = null } }
  function tick() {
    frame = requestAnimationFrame(tick)
    if (!enabled.value) return
    position.value = time()
    active.value = activeCueIndex(transcript.value?.cues || [], position.value - offset.value)
    if (originalEnd !== null && position.value >= originalEnd) {
      originalEnd = null; const next = afterOriginal; afterOriginal = null
      void pause().then(() => next?.()).catch(c => { error.value = String(c) }); return
    }
    if (executing || recording.value || savingRecording.value) return
    const playing = browser.value ? !browser.value.paused() : playback.isPlaying
    const ended = browser.value ? browser.value.ended() : playback.session?.status === 'ended'
    if (playing && active.value >= 0 && selected.value !== active.value) {
      selected.value = active.value; explanation.value = ''
      selectedRecording.value = recordings.value.find(r => r.cue_id === cue.value?.id)?.id || null
    }
    const action = loop?.tick(position.value, playing || ended, performance.now())
    if (action) {
      const token = operation; executing = true
      void (async () => {
        if (action.kind === 'play') { await seekTo(action.position); if (token === operation) await play() }
        else { await pause(); if (action.kind === 'finished') stopLoop() }
      })().catch(c => { error.value = String(c); stopLoop() }).finally(() => { if (token === operation) executing = false })
    } else if (!loop && skipSilence.value && playing && active.value < 0 && transcript.value && originalEnd === null) {
      const next = transcript.value.cues.find(c => c.start + offset.value > position.value + 0.1 && c.start + offset.value < playback.duration)
      if (next) { executing = true; void seekTo(next.start + offset.value).catch(c => { error.value = String(c) }).finally(() => { executing = false }) }
    }
  }
  async function selectTranscript(value: Transcript) {
    if (recording.value || recordingStarting.value || savingRecording.value || pendingRecording.value) throw new Error('请先停止并保存当前录音')
    stopLoop(); stopPreview(); transcript.value = value; selected.value = 0; offset.value = 0; explanation.value = ''; a.value = null; b.value = null
    const token = generation
    const list = await invoke<Recording[]>('list_learning_recordings', { transcriptId: value.id })
    if (token === generation && transcript.value?.id === value.id) { recordings.value = list; selectedRecording.value = null }
  }
  async function loadMedia() {
    const token = ++generation; stopLoop(); stopPreview(); error.value = ''
    transcript.value = null; transcripts.value = []; recordings.value = []; jobs.value = []; explanation.value = ''; a.value = null; b.value = null
    const mediaId = playback.session?.media.id
    if (!enabled.value || !mediaId) return
    loading.value = true
    try {
      const [list, pending] = await Promise.all([invoke<Transcript[]>('list_learning_transcripts', { mediaId }), invoke<string[]>('pending_learning_transcriptions', { mediaId })])
      if (token !== generation) return
      transcripts.value = list; jobs.value = pending
      if (list.length) await selectTranscript(list[0])
      else {
        const assets = await invoke<MediaAsset[]>('get_media_subtitles', { mediaId })
        if (token !== generation) return
        if (assets.length) {
          const result = await invoke<Transcript>('import_learning_subtitle', { mediaId, path: assets[0].path })
          if (token === generation) { transcripts.value = [result]; await selectTranscript(result) }
        }
      }
    } catch (cause) { if (token === generation) error.value = String(cause) }
    finally { if (token === generation) loading.value = false }
  }
  async function toggle() {
    if (recording.value || recordingStarting.value || savingRecording.value || pendingRecording.value) throw new Error('请先停止并保存当前录音')
    enabled.value = !enabled.value; playback.learningActive = enabled.value
    stopLoop(); stopPreview()
    if (enabled.value) { await initialize(); defaults(); await loadMedia(); if (!frame) frame = requestAnimationFrame(tick) }
    else { cancelAnimationFrame(frame); frame = 0; generation++; if (browser.value) browser.value.playbackRate(1) }
  }
  async function importSubtitle(path: string) {
    const mediaId = playback.session?.media.id, token = generation
    if (!mediaId) throw new Error('请先打开媒体')
    const result = await invoke<Transcript>('import_learning_subtitle', { mediaId, path })
    if (token === generation) { transcripts.value = [result, ...transcripts.value.filter(t => t.id !== result.id)]; await selectTranscript(result) }
  }
  async function favorite(index: number) {
    const t = transcript.value, c = t?.cues[index]
    if (!t || !c) return
    const value = !c.favorite
    await invoke('set_learning_favorite', { transcriptId: t.id, cueId: c.id, favorite: value }); c.favorite = value
  }
  async function explain(translate = false) {
    const t = transcript.value, c = cue.value, token = generation
    if (!t || !c || busy.value) return
    busy.value = true
    try {
      const result = await invoke<{ text: string; language: string }>('explain_learning_cue', { transcriptId: t.id, cueId: c.id, translate })
      if (token === generation && transcript.value?.id === t.id) {
        if (translate) { c.translation = result.text; c.translation_language = result.language }
        else if (cue.value?.id === c.id) explanation.value = result.text
      }
    } finally { busy.value = false }
  }
  async function startRecording() {
    if (recordingStarting.value) return
    recordingStarting.value = true
    try { await captureRecording() } finally { recordingStarting.value = false }
  }
  async function captureRecording() {
    if (!cue.value || !transcript.value || !canControl.value || recording.value || savingRecording.value) return
    if (pendingRecording.value) throw new Error('请先保存或丢弃上一次录音')
    if (!settings.value?.recording_directory) throw new Error('请先在设置 → 录音中选择保存目录')
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === 'undefined') throw new Error('当前 WebView 不支持录音，请更新系统 WebView')
    const target = { transcriptId: transcript.value.id, cueId: cue.value.id }, token = generation
    stopLoop(); stopPreview(); await pause()
    if (token !== generation) return
    const device = settings.value.recording_device
    const stream = await navigator.mediaDevices.getUserMedia({ audio: device === 'default' ? true : { deviceId: { exact: device } }, video: false })
    if (token !== generation) { stream.getTracks().forEach(t => t.stop()); return }
    input = stream; recordingTarget.value = target
    const chunks: Blob[] = []
    try { recorder = new MediaRecorder(stream) }
    catch (cause) { stream.getTracks().forEach(t => t.stop()); input = null; recordingTarget.value = null; throw cause }
    stream.getAudioTracks().forEach(track => { track.onended = stopRecording })
    recorder.ondataavailable = e => { if (e.data.size) chunks.push(e.data) }
    recorder.onerror = () => { error.value = '录音设备错误，请重新选择麦克风'; stopRecording() }
    recorder.onstop = () => {
      input?.getTracks().forEach(t => t.stop()); input = null; recorder = null
      clearInterval(recordingTimer); recording.value = false
      if (chunks.length) { pendingRecording.value = new Blob(chunks, { type: chunks[0].type }); void saveRecording().catch(c => { error.value = String(c) }) }
      else error.value = '没有录到音频，请检查麦克风权限和输入设备'
    }
    try { recorder.start(250); recording.value = true; recordingSeconds.value = 0 }
    catch (cause) { stream.getTracks().forEach(t => t.stop()); recorder = null; input = null; throw cause }
    recordingTimer = setInterval(() => { recordingSeconds.value++; if (recordingSeconds.value >= 180) stopRecording() }, 1000)
  }
  function stopRecording() { if (recorder && recorder.state !== 'inactive') recorder.stop() }
  async function saveRecording() {
    const blob = pendingRecording.value, target = recordingTarget.value
    if (!blob || !target || savingRecording.value) return
    savingRecording.value = true
    try {
      const result = await invoke<Recording>('save_learning_recording', { transcriptId: target.transcriptId, cueId: target.cueId, bytes: Array.from(new Uint8Array(await blob.arrayBuffer())) })
      pendingRecording.value = null; recordingTarget.value = null
      if (transcript.value?.id === result.transcript_id) { recordings.value.unshift(result); selectedRecording.value = result.id }
    } finally { savingRecording.value = false }
  }
  function discardRecording() { if (!savingRecording.value && !recording.value) { pendingRecording.value = null; recordingTarget.value = null } }
  async function listenRecording(item = currentRecording.value) {
    if (!item) return
    stopLoop(); stopPreview(); await pause()
    preview = new Audio(convertFileSrc(item.path)); await preview.play()
    preview.onerror = () => { error.value = '录音无法回放'; stopPreview() }
  }
  async function compare() {
    const item = currentRecording.value
    const c = transcript.value?.cues.find(c => c.id === item?.cue_id)
    if (!item || !c) return
    const segment = cueSegment(c, offset.value, playback.duration)
    stopLoop(); stopPreview(); await pause()
    const token = operation, mediaToken = generation
    const runRecording = async (next?: () => void) => {
      if (token !== operation || mediaToken !== generation) return
      preview = new Audio(convertFileSrc(item.path)); preview.onended = () => next?.(); preview.onerror = () => { error.value = '录音无法回放'; stopPreview() }; await preview.play()
    }
    const runOriginal = async (next?: () => void) => {
      if (token !== operation || mediaToken !== generation) return
      await seekTo(segment.start)
      if (token !== operation || mediaToken !== generation) return
      originalEnd = segment.end; afterOriginal = next || null; await play()
    }
    if (settings.value?.playback_order === 'recording_first') await runRecording(() => { void runOriginal().catch(c => { error.value = String(c) }) })
    else await runOriginal(() => { void runRecording().catch(c => { error.value = String(c) }) })
  }
  async function deleteRecording(id: string) { stopLoop(); stopPreview(); await invoke('delete_learning_recording', { id }); recordings.value = recordings.value.filter(r => r.id !== id); if (selectedRecording.value === id) selectedRecording.value = null }
  async function evaluate() {
    const item = currentRecording.value
    if (!item || busy.value) return
    busy.value = true
    try { item.evaluation = await invoke<string>('evaluate_learning_recording', { id: item.id }) } finally { busy.value = false }
  }
  async function transcribe(source: string) {
    const mediaId = playback.session?.media.id, token = generation
    if (!mediaId || busy.value) return
    busy.value = true
    try { const id = await invoke<string>('start_learning_transcription', { mediaId, source }); if (token === generation) jobs.value.push(id) } finally { busy.value = false }
  }
  async function pollJob(id: string) {
    const token = generation
    let result: Transcript | null
    try { result = await invoke<Transcript | null>('poll_learning_transcription', { id }) }
    catch (cause) {
      const mediaId = playback.session?.media.id
      if (token === generation && mediaId) jobs.value = await invoke<string[]>('pending_learning_transcriptions', { mediaId })
      throw cause
    }
    if (token !== generation) return
    if (result) {
      jobs.value = jobs.value.filter(j => j !== id); transcripts.value = [result, ...transcripts.value.filter(t => t.id !== result.id)]
      if (!recording.value && !recordingStarting.value && !savingRecording.value && !pendingRecording.value) await selectTranscript(result)
    }
  }
  watch(rate, value => { stopLoop(); if (browser.value && enabled.value) browser.value.playbackRate(value) })
  watch([repetitions, gap, offset], stopLoop)
  watch(() => playback.session?.id, () => { stopRecording(); browser.value = null; browserSession.value = null; if (enabled.value) void loadMedia() })
  function dispose() { stopRecording(); stopLoop(); stopPreview(); unlisten?.(); cancelAnimationFrame(frame); generation++ }
  return { settings, enabled, error, busy, loading, transcripts, transcript, selected, active, position, cue, recordings, selectedRecording, currentRecording,
    repeat, ab, a, b, rate, repetitions, gap, skipSilence, masked, bilingual, favoritesOnly, offset, explanation, jobs,
    recording, recordingStarting, recordingSeconds, savingRecording, pendingRecording, canControl,
    initialize, accept, toggle, attach, detach, stopLoop, stopPreview, jump, toggleRepeat, markA, markB, play, pause, seekTo,
    selectTranscript, loadMedia, importSubtitle, favorite, explain, startRecording, stopRecording, saveRecording, discardRecording,
    listenRecording, compare, deleteRecording, evaluate, transcribe, pollJob, dispose }
})
