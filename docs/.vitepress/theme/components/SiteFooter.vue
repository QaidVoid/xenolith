<script setup lang="ts">
import { useData } from 'vitepress'

const { theme } = useData()
</script>

<template>
  <footer class="footer">
    <p>
      Dual licensed under
      <a href="https://opensource.org/license/mit">MIT</a> or
      <a href="https://opensource.org/license/apache-2-0">Apache 2.0</a>.
    </p>
    <p>
      No game data is in this project, and no key material ships with it.
      <a :href="theme.repository" rel="noreferrer">Source</a>.
    </p>
  </footer>
</template>
