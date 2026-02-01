<script setup lang="ts">
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ExternalLink, X, RefreshCw, KeyRound } from 'lucide-vue-next'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()

const props = defineProps<{
  show: boolean
  platform: string
  loginUrl: string
  originalUrl: string
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'retry', url: string): void
}>()

const isProcessing = ref(false)
const oauthError = ref('')

const platformIcon = computed(() => {
  switch (props.platform.toLowerCase()) {
    case 'youtube':
      return '🎬'
    case 'bilibili':
    case '哔哩哔哩':
      return '📺'
    case 'douyin':
    case '抖音':
      return '🎵'
    default:
      return '🔐'
  }
})

// Check if OAuth is supported for this platform
const supportsOAuth = computed(() => {
  return props.platform.toLowerCase() === 'youtube'
})

async function tryOAuth() {
  if (!supportsOAuth.value) return

  isProcessing.value = true
  oauthError.value = ''

  try {
    await invoke('add_url_with_oauth', { url: props.originalUrl })
    // If successful, close dialog and retry
    emit('retry', props.originalUrl)
    emit('close')
  } catch (e: any) {
    console.error('OAuth failed:', e)
    oauthError.value = String(e)
  } finally {
    isProcessing.value = false
  }
}

async function openLogin() {
  isProcessing.value = true
  try {
    await invoke('open_platform_login', { platform: props.platform })
  } catch (e) {
    console.error('Failed to open login page:', e)
  } finally {
    isProcessing.value = false
  }
}

function retry() {
  emit('retry', props.originalUrl)
  emit('close')
}

function close() {
  emit('close')
}
</script>

<template>
  <Teleport to="body">
    <div v-if="show" class="fixed inset-0 z-50 flex items-center justify-center">
      <!-- Backdrop -->
      <div class="absolute inset-0 bg-black/60 backdrop-blur-sm" @click="close"></div>

      <!-- Dialog -->
      <div class="relative bg-zinc-900 rounded-xl shadow-2xl border border-zinc-700 w-[450px] max-w-[90vw] overflow-hidden">
        <!-- Header -->
        <div class="flex items-center justify-between px-5 py-4 border-b border-zinc-700">
          <div class="flex items-center gap-3">
            <span class="text-2xl">{{ platformIcon }}</span>
            <h3 class="text-lg font-semibold text-white">{{ t('login.required') }}</h3>
          </div>
          <button
            @click="close"
            class="p-1.5 rounded-lg hover:bg-zinc-700 text-zinc-400 hover:text-white transition-colors"
          >
            <X class="w-5 h-5" />
          </button>
        </div>

        <!-- Content -->
        <div class="px-5 py-5 space-y-4">
          <p class="text-zinc-300 text-sm leading-relaxed">
            {{ t('login.message', { platform }) }}
          </p>

          <!-- OAuth Option (Recommended for YouTube) -->
          <div v-if="supportsOAuth" class="bg-blue-900/20 border border-blue-700/50 rounded-lg p-4">
            <div class="flex items-center gap-2 mb-2">
              <KeyRound class="w-4 h-4 text-blue-400" />
              <span class="text-blue-400 font-medium text-sm">{{ t('login.oauthRecommended') }}</span>
            </div>
            <p class="text-zinc-400 text-xs mb-3">
              {{ t('login.oauthDesc') }}
            </p>
            <button
              @click="tryOAuth"
              :disabled="isProcessing"
              class="w-full flex items-center justify-center gap-2 px-4 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:bg-blue-600/50 text-white rounded-lg font-medium transition-colors"
            >
              <KeyRound class="w-4 h-4" />
              {{ isProcessing ? t('login.authorizing') : t('login.oauthButton') }}
            </button>
            <p v-if="oauthError" class="text-red-400 text-xs mt-2">{{ oauthError }}</p>
          </div>

          <!-- Manual Login Steps -->
          <div class="bg-zinc-800/50 rounded-lg p-4 space-y-3">
            <div class="text-zinc-400 text-xs font-medium mb-2">{{ t('login.manualSteps') }}</div>
            <div class="flex items-start gap-2 text-sm text-zinc-400">
              <span class="text-amber-500 mt-0.5">1.</span>
              <span>{{ t('login.step1') }}</span>
            </div>
            <div class="flex items-start gap-2 text-sm text-zinc-400">
              <span class="text-amber-500 mt-0.5">2.</span>
              <span>{{ t('login.step2') }}</span>
            </div>
            <div class="flex items-start gap-2 text-sm text-zinc-400">
              <span class="text-amber-500 mt-0.5">3.</span>
              <span>{{ t('login.step3') }}</span>
            </div>
          </div>
        </div>

        <!-- Actions -->
        <div class="flex gap-3 px-5 py-4 bg-zinc-800/30 border-t border-zinc-700">
          <button
            @click="openLogin"
            :disabled="isProcessing"
            class="flex-1 flex items-center justify-center gap-2 px-4 py-2.5 bg-zinc-700 hover:bg-zinc-600 disabled:bg-zinc-700/50 text-white rounded-lg font-medium transition-colors"
          >
            <ExternalLink class="w-4 h-4" />
            {{ t('login.openBrowser') }}
          </button>
          <button
            @click="retry"
            :disabled="isProcessing"
            class="flex items-center justify-center gap-2 px-4 py-2.5 bg-zinc-700 hover:bg-zinc-600 disabled:bg-zinc-700/50 text-white rounded-lg font-medium transition-colors"
          >
            <RefreshCw class="w-4 h-4" />
            {{ t('login.retry') }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
