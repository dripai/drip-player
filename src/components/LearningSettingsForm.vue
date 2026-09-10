<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { useLearningStore, type LearningSettings } from '../store/learning'

const props = defineProps<{ section: 'models' | 'subtitles' | 'shadow' | 'recording' }>()
const learning = useLearningStore()
const draft = ref<LearningSettings | null>(null)
const error = ref(''), message = ref(''), saving = ref(false), dirty = ref(false)
const dashscope = ref(''), apiKey = ref(''), apiSecret = ref('')
const secrets = ref({ dashscope: false, xfyun: false })
const devices = ref<MediaDeviceInfo[]>([])
const titles = { models: '模型服务', subtitles: '字幕', shadow: '影子跟读', recording: '录音' }
const deviceMissing = computed(() => draft.value?.recording_device !== 'default' && !devices.value.some(d => d.deviceId === draft.value?.recording_device))

function reload() {
  if (learning.settings) draft.value = { ...learning.settings }
  dirty.value = false; error.value = ''; message.value = ''
}
async function refreshSecrets() {
  if (props.section !== 'models') return
  try { secrets.value = await invoke('learning_secret_status') } catch (cause) { error.value = String(cause) }
}
async function fetchSettings() {
  try { learning.accept(await invoke('get_learning_settings')); reload() } catch (cause) { error.value = String(cause) }
}
watch(() => props.section, () => { reload(); dashscope.value = ''; apiKey.value = ''; apiSecret.value = ''; void refreshSecrets() })
watch(() => learning.settings?.revision, () => { if (!dirty.value) reload() })
onMounted(async () => {
  await learning.initialize(); reload()
  await refreshSecrets()
})
async function save() {
  if (!draft.value) return
  saving.value = true; error.value = ''; message.value = ''
  try {
    learning.accept(await invoke('update_learning_settings', { value: { ...draft.value } }))
    reload(); message.value = '已保存'
  } catch (cause) { error.value = String(cause) } finally { saving.value = false }
}
async function saveSecret(kind: 'dashscope' | 'xfyun', clear = false) {
  const value = clear ? '' : kind === 'dashscope' ? dashscope.value.trim() : JSON.stringify({ api_key: apiKey.value.trim(), api_secret: apiSecret.value.trim() })
  if (!clear && (kind === 'dashscope' ? !dashscope.value.trim() : !apiKey.value.trim() || !apiSecret.value.trim())) { error.value = '请填写完整凭据'; return }
  saving.value = true; error.value = ''; message.value = ''
  try {
    await invoke('save_learning_secret', { kind, value })
    secrets.value[kind] = !clear; dashscope.value = ''; apiKey.value = ''; apiSecret.value = ''
    message.value = clear ? '凭据已移除' : '凭据已存入系统凭据库'
  } catch (cause) { error.value = String(cause) } finally { saving.value = false }
}
async function chooseDirectory() {
  try {
    const path = await open({ directory: true, multiple: false, title: '选择录音保存目录' })
    if (typeof path === 'string' && draft.value) { draft.value.recording_directory = path; dirty.value = true }
  } catch (cause) { error.value = String(cause) }
}
async function refreshDevices() {
  let stream: MediaStream | undefined
  try {
    if (!navigator.mediaDevices?.getUserMedia) throw new Error('当前 WebView 不支持麦克风访问')
    stream = await navigator.mediaDevices.getUserMedia({ audio: true, video: false })
    devices.value = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === 'audioinput' && d.deviceId !== 'default')
    error.value = ''
  } catch (cause) { error.value = String(cause) } finally { stream?.getTracks().forEach(t => t.stop()) }
}
</script>

<template>
  <div>
    <h1 class="text-xl font-semibold">{{ titles[section] }}</h1>
    <form v-if="draft" class="mt-5" @submit.prevent="save" @input="dirty = true" @change="dirty = true">
      <fieldset :disabled="saving" class="disabled:opacity-60">
        <template v-if="section === 'models'">
          <section class="provider-section">
            <div class="provider-heading">
              <h2>百炼 · 北京地域</h2>
              <span class="credential-status">{{ secrets.dashscope ? 'Key 已配置' : 'Key 未配置' }}</span>
            </div>
            <div class="form-fields">
              <div class="form-row">
                <label for="learning-workspace">业务空间 ID</label>
                <input id="learning-workspace" v-model.trim="draft.workspace_id" autocomplete="off" placeholder="Workspace ID" />
              </div>
              <div class="form-row">
                <label for="learning-text-model">翻译 / 讲解模型</label>
                <input id="learning-text-model" v-model.trim="draft.text_model" placeholder="qwen-plus" />
              </div>
              <div class="form-row">
                <label for="learning-transcription-model">字幕识别模型</label>
                <input id="learning-transcription-model" :value="draft.transcription_model" readonly />
              </div>
              <div class="form-row">
                <label for="learning-dashscope-key">API Key</label>
                <div class="control-actions">
                  <input id="learning-dashscope-key" v-model="dashscope" type="password" autocomplete="new-password" placeholder="填写新 Key" />
                  <button type="button" class="secondary" aria-label="保存百炼 Key" @click="saveSecret('dashscope')">保存 Key</button>
                  <button v-if="secrets.dashscope" type="button" class="secondary" aria-label="移除百炼 Key" @click="saveSecret('dashscope', true)">移除</button>
                </div>
              </div>
            </div>
          </section>
          <section class="provider-section">
            <div class="provider-heading">
              <h2>讯飞 · 英语语音评测</h2>
              <span class="credential-status">{{ secrets.xfyun ? '凭据已配置' : '凭据未配置' }}</span>
            </div>
            <div class="form-fields">
              <div class="form-row">
                <label for="learning-xfyun-app-id">APPID</label>
                <input id="learning-xfyun-app-id" v-model.trim="draft.evaluation_app_id" autocomplete="off" />
              </div>
              <div class="form-row">
                <label for="learning-xfyun-key">APIKey</label>
                <input id="learning-xfyun-key" v-model="apiKey" type="password" autocomplete="new-password" />
              </div>
              <div class="form-row">
                <label for="learning-xfyun-secret">APISecret</label>
                <div class="control-actions">
                  <input id="learning-xfyun-secret" v-model="apiSecret" type="password" autocomplete="new-password" />
                  <button type="button" class="secondary" aria-label="保存讯飞凭据" @click="saveSecret('xfyun')">保存凭据</button>
                  <button v-if="secrets.xfyun" type="button" class="secondary" aria-label="移除讯飞凭据" @click="saveSecret('xfyun', true)">移除</button>
                </div>
              </div>
            </div>
          </section>
        </template>
        <div v-else-if="section === 'subtitles'" class="form-fields">
          <div class="form-row">
            <label for="learning-subtitle-display">默认显示</label>
            <select id="learning-subtitle-display" v-model="draft.subtitle_display"><option value="bilingual">双语</option><option value="original">原文</option></select>
          </div>
          <div class="form-row">
            <label for="learning-translation-language">翻译目标语言</label>
            <select id="learning-translation-language" v-model="draft.translation_language"><option value="zh">中文</option><option value="en">英语</option><option value="ja">日语</option><option value="ko">韩语</option></select>
          </div>
          <div class="form-row">
            <label for="learning-subtitle-font-size">字号</label>
            <input id="learning-subtitle-font-size" v-model.number="draft.subtitle_font_size" class="number-input" type="number" min="12" max="36" />
          </div>
          <div class="form-row">
            <label for="learning-auto-scroll">字幕列表自动跟随播放</label>
            <input id="learning-auto-scroll" v-model="draft.auto_scroll" type="checkbox" />
          </div>
        </div>
        <div v-else-if="section === 'shadow'" class="form-fields">
          <div class="form-row">
            <label for="learning-playback-rate">默认视频倍速</label>
            <select id="learning-playback-rate" v-model.number="draft.playback_rate"><option v-for="value in [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2]" :key="value" :value="value">{{ value }}×</option></select>
          </div>
          <div class="form-row">
            <label for="learning-repetitions">每句播放次数</label>
            <input id="learning-repetitions" v-model.number="draft.repetitions" class="number-input" type="number" min="1" max="10" />
          </div>
          <div class="form-row">
            <label for="learning-gap-seconds">跟读停顿（秒）</label>
            <input id="learning-gap-seconds" v-model.number="draft.gap_seconds" class="number-input" type="number" min="0" max="10" step="0.5" />
          </div>
          <div class="form-row">
            <label for="learning-skip-silence">跳过字幕句间空白</label>
            <input id="learning-skip-silence" v-model="draft.skip_silence" type="checkbox" />
          </div>
        </div>
        <div v-else class="form-fields">
          <div class="form-row">
            <label for="learning-recording-device">麦克风</label>
            <div class="control-actions">
              <select id="learning-recording-device" v-model="draft.recording_device"><option value="default">系统默认输入</option><option v-if="deviceMissing" :value="draft.recording_device">已保存的设备</option><option v-for="device in devices" :key="device.deviceId" :value="device.deviceId">{{ device.label || '未命名麦克风' }}</option></select>
              <button type="button" class="secondary" aria-label="刷新麦克风" @click="refreshDevices">刷新</button>
            </div>
          </div>
          <div class="form-row">
            <label for="learning-recording-directory">保存目录</label>
            <div class="control-actions">
              <input id="learning-recording-directory" v-model.trim="draft.recording_directory" readonly placeholder="请选择目录" />
              <button type="button" class="secondary" aria-label="选择目录" @click="chooseDirectory">选择</button>
            </div>
          </div>
          <div class="form-row">
            <label for="learning-playback-order">原声对比顺序</label>
            <select id="learning-playback-order" v-model="draft.playback_order"><option value="original_first">原声 → 录音</option><option value="recording_first">录音 → 原声</option></select>
          </div>
        </div>
      </fieldset>
      <div class="form-actions">
        <button type="submit" :disabled="saving" class="h-9 shrink-0 rounded-md bg-blue-600 px-4 text-sm text-white hover:bg-blue-700 disabled:opacity-50">{{ saving ? '保存中…' : '保存设置' }}</button>
        <button type="button" class="secondary" :disabled="saving" @click="fetchSettings">重新载入</button>
        <span role="status" class="text-sm text-green-600 dark:text-green-400">{{ message }}</span>
      </div>
    </form>
    <p v-else class="mt-5 text-sm text-red-600">{{ learning.error || '正在加载设置…' }}</p>
    <p v-if="error" role="alert" class="mt-4 break-words rounded-lg bg-red-50 p-3 text-sm text-red-700 dark:bg-red-950 dark:text-red-200">{{ error }}</p>
  </div>
</template>

<style scoped>
@reference "../style.css";
.provider-section + .provider-section { @apply mt-5 border-t border-zinc-200 pt-5 dark:border-zinc-800; }
.provider-heading { @apply mb-3 flex items-center justify-between gap-3; }
.credential-status { @apply text-xs text-zinc-500 dark:text-zinc-400; }
.form-fields { @apply space-y-3; }
.form-row { @apply grid min-h-9 items-center gap-x-4; grid-template-columns: 184px minmax(0, 1fr); }
.control-actions { @apply flex min-w-0 items-center gap-2; }
.control-actions > input, .control-actions > select { @apply min-w-0 flex-1; }
.form-actions { @apply mt-6 flex flex-wrap items-center gap-3; padding-left: 200px; }
h2 { @apply text-sm font-semibold; }
label { @apply text-sm; }
input:not([type='checkbox']), select { @apply h-9 w-full min-w-0 rounded-md border border-zinc-300 bg-white px-3 text-sm outline-hidden focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-zinc-600 dark:bg-zinc-800; }
input.number-input { @apply w-28; }
input[type='checkbox'] { @apply h-4 w-4 cursor-pointer accent-blue-600; }
.secondary { @apply h-9 shrink-0 whitespace-nowrap rounded-md border border-zinc-300 px-3 text-sm hover:bg-zinc-100 disabled:opacity-50 dark:border-zinc-600 dark:hover:bg-zinc-700; }
</style>
