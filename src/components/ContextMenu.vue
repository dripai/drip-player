<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'

interface MenuItem {
  label: string
  icon?: any
  action: () => void
  danger?: boolean
}

interface Props {
  x: number
  y: number
  items: MenuItem[]
}

const props = defineProps<Props>()
const emit = defineEmits<{
  close: []
}>()

const menuRef = ref<HTMLElement | null>(null)

function handleClickOutside(e: MouseEvent) {
  if (menuRef.value && !menuRef.value.contains(e.target as Node)) {
    emit('close')
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
  document.addEventListener('contextmenu', handleClickOutside)
})

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
  document.removeEventListener('contextmenu', handleClickOutside)
})
</script>

<template>
  <div
    ref="menuRef"
    class="fixed z-50 bg-white dark:bg-zinc-800 border dark:border-zinc-700 rounded-lg shadow-xl py-1 min-w-[180px]"
    :style="{ left: x + 'px', top: y + 'px' }"
  >
    <button
      v-for="(item, index) in items"
      :key="index"
      @click="item.action(); emit('close')"
      class="w-full px-4 py-2 text-left text-sm hover:bg-zinc-100 dark:hover:bg-zinc-700 flex items-center gap-3 transition-colors"
      :class="{ 'text-red-600 dark:text-red-400': item.danger }"
    >
      <component v-if="item.icon" :is="item.icon" class="w-4 h-4" />
      <span>{{ item.label }}</span>
    </button>
  </div>
</template>
