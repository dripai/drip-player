<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { closeContextMenu, contextMenu, type ContextMenuItem } from '../composables/contextMenu'

const menu = ref<HTMLDivElement | null>(null)
const left = ref(0)
const top = ref(0)
const emit = defineEmits<{ error: [message: string] }>()

watch(contextMenu, async state => {
  if (!state) return
  await nextTick()
  if (state !== contextMenu.value || !menu.value) return
  const bounds = menu.value.getBoundingClientRect()
  left.value = Math.max(8, Math.min(state.x, window.innerWidth - bounds.width - 8))
  top.value = Math.max(8, Math.min(state.y, window.innerHeight - bounds.height - 8))
  buttons()[0]?.focus({ preventScroll: true })
}, { flush: 'post' })

function buttons() { return Array.from(menu.value?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') || []) }
function outside(event: Event) {
  if (contextMenu.value && event.target instanceof Node && !menu.value?.contains(event.target)) closeContextMenu()
}
function keydown(event: KeyboardEvent) {
  const options = buttons()
  const index = options.indexOf(document.activeElement as HTMLButtonElement)
  if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    event.preventDefault()
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1
      : (index + (event.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length
    options[next]?.focus()
  } else if (event.key === 'Escape' || event.key === 'Tab') {
    event.preventDefault()
    closeContextMenu()
  }
}
async function select(item: ContextMenuItem) {
  if (item.disabled) return
  closeContextMenu()
  try { await item.action() } catch (cause) { emit('error', String(cause)) }
}
onMounted(() => {
  document.addEventListener('pointerdown', outside, true)
  document.addEventListener('scroll', outside, true)
  window.addEventListener('blur', closeContextMenu)
  window.addEventListener('resize', closeContextMenu)
})
onUnmounted(() => {
  document.removeEventListener('pointerdown', outside, true)
  document.removeEventListener('scroll', outside, true)
  window.removeEventListener('blur', closeContextMenu)
  window.removeEventListener('resize', closeContextMenu)
  closeContextMenu()
})
</script>

<template>
  <Teleport to="body">
    <div v-if="contextMenu" ref="menu" role="menu" class="fixed z-[100] min-w-48 max-w-[calc(100vw-16px)] rounded-lg border border-zinc-200 bg-white p-1 text-sm text-zinc-800 shadow-lg dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-100"
      :style="{ left: `${left}px`, top: `${top}px` }" @keydown="keydown" @contextmenu.prevent.stop>
      <template v-for="item in contextMenu.items" :key="item.id">
        <div v-if="item.separatorBefore" role="separator" class="mx-2 my-1 border-t border-zinc-200 dark:border-zinc-700" />
        <button type="button" role="menuitem" tabindex="-1" :disabled="item.disabled" @click="select(item)"
          class="flex min-h-8 w-full items-center gap-2.5 rounded px-2.5 py-1.5 text-left outline-none hover:bg-zinc-100 focus:bg-zinc-100 disabled:opacity-40 dark:hover:bg-zinc-800 dark:focus:bg-zinc-800"
          :class="item.danger ? 'text-red-600 dark:text-red-400' : ''">
          <component :is="item.icon" v-if="item.icon" class="h-4 w-4 shrink-0" aria-hidden="true" />
          <span>{{ item.label }}</span>
        </button>
      </template>
    </div>
  </Teleport>
</template>
