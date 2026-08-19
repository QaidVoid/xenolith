<script setup lang="ts">
import { useRoute } from 'vitepress'
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'

interface Entry {
  id: string
  text: string
  level: number
}

const route = useRoute()
const entries = ref<Entry[]>([])
const active = ref('')

let observer: IntersectionObserver | undefined

/**
 * Builds the outline from the rendered page rather than from page data.
 *
 * The headings that exist are the ones in the document, which is the thing a
 * reader is looking at, so reading them from it cannot disagree with it.
 */
function collect() {
  const article = document.querySelector('.content')
  if (!article) {
    entries.value = []
    return
  }

  const headings = Array.from(article.querySelectorAll<HTMLElement>('h2, h3'))
  entries.value = headings
    .filter((heading) => heading.id)
    .map((heading) => ({
      id: heading.id,
      text: heading.textContent?.replace(/#$/, '').trim() ?? '',
      level: Number(heading.tagName.slice(1)),
    }))

  observer?.disconnect()
  if (entries.value.length === 0) return

  observer = new IntersectionObserver(
    (records) => {
      for (const record of records) {
        if (record.isIntersecting) {
          active.value = record.target.id
          break
        }
      }
    },
    { rootMargin: '-72px 0px -70% 0px', threshold: 0 },
  )
  for (const heading of headings) {
    if (heading.id) observer.observe(heading)
  }
}

onMounted(() => {
  void nextTick(collect)
})
onUnmounted(() => observer?.disconnect())
watch(
  () => route.path,
  () => void nextTick(collect),
)
</script>

<template>
  <aside v-if="entries.length > 1" class="toc" aria-label="On this page">
    <p class="toc__label">On this page</p>
    <ul>
      <li v-for="entry in entries" :key="entry.id" :class="`toc__l${entry.level}`">
        <a :href="`#${entry.id}`" :class="{ 'is-current': active === entry.id }">
          {{ entry.text }}
        </a>
      </li>
    </ul>
  </aside>
</template>
