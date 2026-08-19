import { ref } from 'vue'

/** Key VitePress own no-flash script reads, so the two agree on load. */
const STORAGE_KEY = 'vitepress-theme-appearance'

/** Whether the dark palette is showing. */
export const isDark = ref(false)

/** Reads the class the no-flash script already put on the document. */
export function readAppearance() {
  if (typeof document === 'undefined') return
  isDark.value = document.documentElement.classList.contains('dark')
}

/** Switches palette, remembering the choice for the next visit. */
export function toggleAppearance() {
  isDark.value = !isDark.value
  document.documentElement.classList.toggle('dark', isDark.value)
  try {
    localStorage.setItem(STORAGE_KEY, isDark.value ? 'dark' : 'light')
  } catch {
    // A browser refusing storage is a preference not remembered, not an error.
  }
}
