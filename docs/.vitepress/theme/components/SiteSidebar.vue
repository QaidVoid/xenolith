<script setup lang="ts">
import { useData, useRoute } from 'vitepress'
import { computed } from 'vue'

interface Item {
  text: string
  link: string
}

interface Section {
  label: string
  address: string
  items: Item[]
}

const { theme } = useData()
const route = useRoute()

const sections = computed(() => (theme.value.sections ?? []) as Section[])

/** Matches a sidebar link against the page being shown. */
function current(link: string) {
  const path = route.path.replace(/\.html$/, '').replace(/\/$/, '')
  const target = link.replace(/\/$/, '')
  return path === target
}
</script>

<template>
  <nav class="sidebar" aria-label="Sections">
    <div v-for="section in sections" :key="section.label" class="sidebar__group">
      <p class="sidebar__label">
        <span class="sidebar__address">{{ section.address }}</span>
        {{ section.label }}
      </p>
      <ul>
        <li v-for="item in section.items" :key="item.link">
          <a
            :href="item.link"
            :class="{ 'is-current': current(item.link) }"
            :aria-current="current(item.link) ? 'page' : undefined"
          >
            {{ item.text }}
          </a>
        </li>
      </ul>
    </div>
  </nav>
</template>
