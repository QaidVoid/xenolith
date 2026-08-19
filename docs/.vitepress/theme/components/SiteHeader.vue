<script setup lang="ts">
import { useData } from 'vitepress'

import { isDark, toggleAppearance } from '../appearance'

defineProps<{ drawerOpen: boolean }>()
defineEmits<{ (event: 'toggle-drawer'): void }>()

const { site, theme } = useData()
</script>

<template>
  <header class="header">
    <button
      class="header__drawer"
      type="button"
      :aria-expanded="drawerOpen"
      aria-label="Toggle the navigation"
      @click="$emit('toggle-drawer')"
    >
      <span class="header__bar" />
      <span class="header__bar" />
      <span class="header__bar" />
    </button>

    <a class="brand" href="/">
      <span class="brand__mark" aria-hidden="true">
        <svg viewBox="0 0 32 32" width="22" height="22">
          <path
            d="M8 9l16 14M24 9L8 23"
            stroke="currentColor"
            stroke-width="3"
            stroke-linecap="round"
            fill="none"
          />
        </svg>
      </span>
      <span class="brand__name">{{ site.title }}</span>
      <span class="brand__tag">ppc64be &rarr; c</span>
    </a>

    <nav class="header__nav">
      <a href="/guide/">Guide</a>
      <a href="/internals/container">Internals</a>
      <a href="/verification">Verification</a>
      <a href="/reference/cli">Reference</a>
    </nav>

    <div class="header__end">
      <button
        class="header__icon"
        type="button"
        :aria-label="isDark ? 'Use the light palette' : 'Use the dark palette'"
        @click="toggleAppearance"
      >
        <svg v-if="isDark" viewBox="0 0 24 24" width="17" height="17">
          <circle cx="12" cy="12" r="4.5" fill="currentColor" />
          <g stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M12 2v3M12 19v3M2 12h3M19 12h3" />
            <path d="M4.9 4.9l2.1 2.1M17 17l2.1 2.1M19.1 4.9L17 7M7 17l-2.1 2.1" />
          </g>
        </svg>
        <svg v-else viewBox="0 0 24 24" width="17" height="17">
          <path
            d="M20 14.5A8.5 8.5 0 019.5 4a8.5 8.5 0 1010.5 10.5z"
            fill="currentColor"
          />
        </svg>
      </button>

      <a
        class="header__icon"
        :href="theme.repository"
        rel="noreferrer"
        aria-label="Source"
      >
        <svg viewBox="0 0 16 16" width="17" height="17" fill="currentColor">
          <path
            d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38
               0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13
               -.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66
               .07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15
               -.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.4 7.4 0 014 0c1.53-1.04
               2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87
               3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38
               A8.01 8.01 0 0016 8c0-4.42-3.58-8-8-8z"
          />
        </svg>
      </a>
    </div>
  </header>
</template>
