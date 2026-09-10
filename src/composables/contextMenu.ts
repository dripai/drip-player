import { shallowRef, type Component } from 'vue'

export interface ContextMenuItem {
  id: string
  label: string
  icon?: Component
  disabled?: boolean
  danger?: boolean
  separatorBefore?: boolean
  action: () => void | Promise<void>
}

interface ContextMenuState {
  x: number
  y: number
  items: ContextMenuItem[]
  trigger: HTMLElement | null
}

export const contextMenu = shallowRef<ContextMenuState | null>(null)

export function openContextMenu(event: MouseEvent, items: ContextMenuItem[]) {
  event.preventDefault()
  event.stopPropagation()
  const trigger = event.target instanceof Element ? event.target.closest<HTMLElement>('[tabindex], button') : null
  const bounds = trigger?.getBoundingClientRect()
  const keyboard = event.clientX === 0 && event.clientY === 0 && bounds
  contextMenu.value = { x: keyboard ? bounds.left : event.clientX, y: keyboard ? bounds.bottom : event.clientY, items, trigger }
}

export function closeContextMenu() {
  const trigger = contextMenu.value?.trigger
  contextMenu.value = null
  trigger?.focus({ preventScroll: true })
}
