<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from 'vue-i18n'
import { Settings, Star, Mic, Square, SkipBack, SkipForward, Repeat, EyeOff, X, Upload, Play, Pause } from '@lucide/vue'
import { useLearningStore } from '../store/learning'
import { usePlayerStore } from '../store/player'

const learning = useLearningStore(), player = usePlayerStore()
const { locale } = useI18n()
const search = ref(''), showSource = ref(false), source = ref(''), list = ref<HTMLElement | null>(null)
const autoScroll = ref(learning.settings?.auto_scroll ?? true)
const locked = computed(() => learning.recording || learning.recordingStarting || learning.savingRecording || Boolean(learning.pendingRecording))
const filtered = computed(() => (learning.transcript?.cues || []).filter(c => (!learning.favoritesOnly || c.favorite) && (!search.value || `${c.text} ${c.translation || ''}`.toLocaleLowerCase().includes(search.value.toLocaleLowerCase()))))
const takes = computed(() => learning.recordings.filter(r => r.cue_id === learning.cue?.id))
const supportsRate = computed(() => player.session?.plan?.engine === 'browser_video')
async function run(action: () => unknown | Promise<unknown>) {
  learning.error = ''
  try { await action() } catch (cause) { learning.error = String(cause) }
}
async function importSubtitle() {
  const path = await open({ multiple: false, title: '导入字幕', filters: [{ name: '字幕', extensions: ['srt', 'vtt', 'ass', 'ssa'] }] })
  if (typeof path === 'string') await learning.importSubtitle(path)
}
function chooseTranscript(event: Event) {
  const value = learning.transcripts.find(t => t.id === (event.target as HTMLSelectElement).value)
  if (value) void run(() => learning.selectTranscript(value))
}
function time(seconds: number) { return `${Math.floor(seconds / 60)}:${Math.floor(seconds % 60).toString().padStart(2, '0')}` }
function recordedAt(seconds: number) { return new Date(seconds * 1000).toLocaleString() }
async function togglePlay() { learning.stopLoop(); learning.stopPreview(); if (player.isPlaying) await learning.pause(); else await learning.play() }
let pollTimer: ReturnType<typeof setInterval> | undefined, polling = false
async function poll() {
  if (polling) return
  polling = true
  try { for (const id of [...learning.jobs]) await learning.pollJob(id) } finally { polling = false }
}
onMounted(() => { pollTimer = setInterval(() => { if (learning.jobs.length && !learning.error) void run(poll) }, 7000) })
onUnmounted(() => clearInterval(pollTimer))
watch(() => learning.active, async index => {
  if (!autoScroll.value || index < 0) return
  await nextTick(); list.value?.querySelector(`[data-cue="${index}"]`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
})
watch(() => learning.settings?.auto_scroll, value => { if (value !== undefined) autoScroll.value = value })
</script>

<template>
  <aside aria-label="影子跟读" class="learning-panel flex h-full w-[390px] max-w-[48vw] shrink-0 flex-col border-l border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-900">
    <header class="flex shrink-0 items-center justify-between border-b border-zinc-200 px-4 py-3 dark:border-zinc-800">
      <h2 class="text-sm font-semibold">影子跟读</h2>
      <div class="flex items-center gap-2"><button class="icon" title="学习设置" aria-label="学习设置" @click="run(() => invoke('open_settings_window', { locale }))"><Settings class="h-4 w-4" /></button><button class="icon" title="退出学习" aria-label="退出学习" @click="run(learning.toggle)"><X class="h-4 w-4" /></button></div>
    </header>
    <div v-if="learning.error" role="alert" class="flex max-h-28 shrink-0 gap-2 overflow-y-auto bg-red-50 p-3 text-xs text-red-700 dark:bg-red-950 dark:text-red-200"><span class="min-w-0 flex-1 break-words">{{ learning.error }}</span><button aria-label="关闭错误提示" class="self-start" @click="learning.error = ''">×</button></div>
    <div v-if="!player.currentTrack" class="p-5 text-sm text-zinc-500">先在播放列表中打开音视频，再进入影子跟读。</div>
    <template v-else>
      <div class="shrink-0 space-y-2 border-b border-zinc-200 p-3 dark:border-zinc-800">
        <div class="flex gap-2">
          <select aria-label="字幕版本" class="min-w-0 flex-1" :value="learning.transcript?.id || ''" :disabled="locked || learning.loading" @change="chooseTranscript"><option v-if="!learning.transcripts.length" value="">{{ learning.loading ? '加载字幕中…' : '尚无字幕' }}</option><option v-for="t in learning.transcripts" :key="t.id" :value="t.id">{{ t.label }} · {{ t.cues.length }} 句</option></select>
          <button class="small" :disabled="locked || learning.loading" @click="run(importSubtitle)">导入字幕</button>
          <button class="small" :disabled="locked" @click="showSource = !showSource">识别</button>
        </div>
        <div v-if="showSource" class="space-y-2 rounded-lg bg-zinc-100 p-2 dark:bg-zinc-800">
          <input v-model.trim="source" aria-label="转写文件 URL" class="w-full" placeholder="可直接下载音视频的 HTTP/HTTPS URL" />
          <div class="flex items-center justify-between gap-2"><span class="text-xs text-zinc-500">百炼云端识别，可能产生费用。</span><button class="small" :disabled="!source || learning.busy || !!learning.jobs.length || locked" @click="run(() => learning.transcribe(source))">生成字幕</button></div>
        </div>
        <div v-if="learning.jobs.length" class="flex items-center justify-between text-xs text-blue-600 dark:text-blue-400"><span>字幕识别处理中…</span><button class="small" @click="run(poll)">查询进度</button></div>
        <div v-if="learning.transcript" class="flex items-center gap-2">
          <input v-model="search" class="min-w-0 flex-1" aria-label="搜索字幕" placeholder="搜索字幕" />
          <button class="icon" :class="{ chosen: learning.favoritesOnly }" :aria-pressed="learning.favoritesOnly" aria-label="只看收藏" title="只看收藏" @click="learning.favoritesOnly = !learning.favoritesOnly"><Star class="h-4 w-4" /></button>
          <button class="icon" :class="{ chosen: learning.masked }" :aria-pressed="learning.masked" aria-label="遮挡字幕" title="遮挡字幕" @click="learning.masked = !learning.masked"><EyeOff class="h-4 w-4" /></button>
          <button class="small" :aria-pressed="learning.bilingual" :class="{ chosen: learning.bilingual }" @click="learning.bilingual = !learning.bilingual">双语</button>
          <label class="flex items-center gap-1 whitespace-nowrap text-xs"><input v-model="autoScroll" type="checkbox" />跟随</label>
        </div>
      </div>
      <div ref="list" class="min-h-20 flex-1 overflow-y-auto" aria-label="字幕时间轴">
        <div v-for="c in filtered" :key="c.id" :data-cue="c.id" class="border-b border-zinc-200/70 px-3 py-3 dark:border-zinc-800" :class="learning.active === c.id ? 'bg-blue-50 dark:bg-blue-950/40' : learning.selected === c.id ? 'bg-zinc-100 dark:bg-zinc-800/70' : ''">
          <div class="mb-1 flex items-center justify-between text-xs text-zinc-500"><span>{{ c.id + 1 }} · {{ time(c.start) }}</span><button class="icon" :class="{ 'text-amber-500': c.favorite }" :aria-label="`${c.favorite ? '取消收藏' : '收藏'}第 ${c.id + 1} 句`" @click="run(() => learning.favorite(c.id))"><Star class="h-3.5 w-3.5" :fill="c.favorite ? 'currentColor' : 'none'" /></button></div>
          <button class="w-full text-left disabled:cursor-default" :disabled="locked || !learning.canControl" :aria-label="`播放第 ${c.id + 1} 句`" @click="run(() => learning.jump(c.id))">
            <span class="block whitespace-pre-line leading-relaxed" :class="learning.active === c.id ? 'text-blue-700 dark:text-blue-300' : ''" :style="{ fontSize: `${learning.settings?.subtitle_font_size || 18}px` }">{{ learning.masked ? '•••' : c.text }}</span>
            <span v-if="learning.bilingual && c.translation && c.translation_language === learning.settings?.translation_language && !learning.masked" class="mt-1 block whitespace-pre-line text-sm leading-relaxed text-zinc-500 dark:text-zinc-400">{{ c.translation }}</span>
          </button>
        </div>
        <p v-if="!learning.transcript && !learning.loading" class="p-5 text-sm leading-6 text-zinc-500">导入 SRT、VTT、ASS 或 SSA 字幕即可逐句练习。也可以在设置中配置百炼后生成字幕。</p>
        <p v-else-if="learning.transcript && !filtered.length" class="p-4 text-sm text-zinc-500">没有匹配的句子。</p>
      </div>
      <div class="max-h-[52%] shrink-0 space-y-3 overflow-y-auto border-t border-zinc-200 p-3 dark:border-zinc-700">
        <p v-if="!learning.canControl" class="text-xs text-amber-600">等待播放器就绪；外部播放器不支持学习控制。</p>
        <fieldset :disabled="locked || !learning.canControl" class="space-y-2">
          <div class="flex items-center justify-between gap-1">
            <button class="icon" aria-label="上一句" :disabled="!learning.cue || learning.selected === 0" @click="run(() => learning.jump(learning.selected - 1))"><SkipBack class="h-4 w-4" /></button>
            <button class="icon" :aria-label="player.isPlaying ? '暂停学习播放' : '开始学习播放'" @click="run(togglePlay)"><Pause v-if="player.isPlaying" class="h-4 w-4" /><Play v-else class="h-4 w-4" /></button>
            <button class="icon" aria-label="下一句" :disabled="!learning.transcript || learning.selected >= learning.transcript.cues.length - 1" @click="run(() => learning.jump(learning.selected + 1))"><SkipForward class="h-4 w-4" /></button>
            <button class="small inline-flex items-center gap-1" :disabled="!learning.cue" :class="{ chosen: learning.repeat }" :aria-pressed="learning.repeat" @click="run(learning.toggleRepeat)"><Repeat class="h-3.5 w-3.5" />逐句复读</button>
            <button class="small" :class="{ chosen: learning.a !== null }" @click="learning.markA">A{{ learning.a !== null ? ` ${time(learning.a)}` : '' }}</button>
            <button class="small" :disabled="learning.a === null" :class="{ chosen: learning.ab }" @click="run(learning.markB)">B{{ learning.b !== null ? ` ${time(learning.b)}` : '' }}</button>
            <button v-if="learning.repeat || learning.ab" class="icon" aria-label="停止复读" @click="learning.stopLoop"><X class="h-3.5 w-3.5" /></button>
          </div>
          <div class="grid grid-cols-3 gap-2 text-xs text-zinc-500">
            <label>倍速<select :value="supportsRate ? learning.rate : 1" class="mt-1 w-full" :disabled="!supportsRate" @change="learning.rate = Number(($event.target as HTMLSelectElement).value)"><option v-for="speed in [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2]" :key="speed" :value="speed">{{ speed }}×</option></select></label>
            <label>每句次数<input v-model.number="learning.repetitions" class="mt-1 w-full" type="number" min="1" max="10" /></label>
            <label>停顿 / 秒<input v-model.number="learning.gap" class="mt-1 w-full" type="number" min="0" max="10" step="0.5" /></label>
          </div>
          <div class="flex items-center justify-between gap-2 text-xs text-zinc-500"><label class="flex items-center gap-1"><input v-model="learning.skipSilence" type="checkbox" />跳过句间空白</label><label class="flex items-center gap-1">字幕偏移 / 秒<input v-model.number="learning.offset" class="w-20" type="number" step="0.1" min="-60" max="60" /></label></div>
        </fieldset>
        <div v-if="learning.cue" class="space-y-2 rounded-lg border border-zinc-200 p-2.5 dark:border-zinc-700">
          <div class="flex items-center justify-between"><span class="text-xs text-zinc-500">第 {{ learning.selected + 1 }} 句 · {{ learning.recording ? `${learning.recordingSeconds}s / 180s` : '跟读录音' }}</span>
            <button v-if="learning.recording" class="small text-red-600" @click="learning.stopRecording"><Square class="mr-1 inline h-3 w-3" />停止并保存</button>
            <button v-else class="small" :disabled="locked || !learning.canControl" @click="run(learning.startRecording)"><Mic class="mr-1 inline h-3.5 w-3.5" />{{ learning.savingRecording ? '保存中…' : '开始录音' }}</button>
          </div>
          <div v-if="takes.length" class="flex gap-2"><select v-model="learning.selectedRecording" class="min-w-0 flex-1" aria-label="本句录音" :disabled="locked"><option :value="null">选择录音</option><option v-for="r in takes" :key="r.id" :value="r.id">{{ recordedAt(r.created_at) }}</option></select></div>
          <div v-if="learning.currentRecording" class="flex flex-wrap gap-2"><button class="small" :disabled="locked" @click="run(() => learning.listenRecording())">试听</button><button class="small" :disabled="locked || !learning.canControl" @click="run(learning.compare)">原声对比</button><button class="small" :disabled="locked || learning.busy" @click="run(learning.evaluate)"><Upload class="mr-1 inline h-3 w-3" />上传评测</button><button class="small" :disabled="locked || learning.busy" @click="run(() => learning.deleteRecording(learning.currentRecording!.id))">删除录音</button></div>
          <p v-if="learning.currentRecording?.evaluation" class="text-xs leading-5 text-blue-600 dark:text-blue-300">讯飞评测：{{ learning.currentRecording.evaluation }}</p>
          <div class="flex gap-2"><button class="small" :disabled="learning.busy || locked" @click="run(() => learning.explain(true))">翻译本句</button><button class="small" :disabled="learning.busy || locked" @click="run(() => learning.explain())">词汇 / 语法讲解</button><span v-if="learning.busy" class="self-center text-xs text-zinc-500">模型处理中…</span></div>
          <p v-if="learning.explanation" class="max-h-40 overflow-y-auto whitespace-pre-wrap text-sm leading-6">{{ learning.explanation }}</p>
        </div>
        <div v-if="learning.pendingRecording && !learning.savingRecording" class="flex items-center justify-between gap-2 text-xs"><span>录音尚未保存</span><button class="small" @click="run(learning.saveRecording)">重试保存</button><button class="small" @click="learning.discardRecording">丢弃录音</button></div>
      </div>
    </template>
  </aside>
</template>

<style scoped>
@reference "../style.css";
.icon { @apply rounded-md p-1.5 hover:bg-zinc-200 disabled:opacity-30 dark:hover:bg-zinc-700; }
.small { @apply whitespace-nowrap rounded-md border border-zinc-300 px-2 py-1.5 text-xs hover:bg-zinc-100 disabled:opacity-40 dark:border-zinc-600 dark:hover:bg-zinc-800; }
.chosen { @apply border-blue-400 bg-blue-100 text-blue-700 dark:bg-blue-950 dark:text-blue-300; }
input:not([type='checkbox']), select { @apply rounded-md border border-zinc-300 bg-white px-2 py-1.5 text-xs outline-hidden focus:border-blue-500 disabled:opacity-40 dark:border-zinc-700 dark:bg-zinc-800; }
input[type='checkbox'] { @apply accent-blue-600; }
</style>
