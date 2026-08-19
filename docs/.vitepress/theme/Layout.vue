<script setup lang="ts">
import { useData, useRoute } from 'vitepress'
import { computed, onMounted, ref, watch } from 'vue'

import HomeHero from './components/HomeHero.vue'
import SiteFooter from './components/SiteFooter.vue'
import SiteHeader from './components/SiteHeader.vue'
import SiteSidebar from './components/SiteSidebar.vue'
import SiteToc from './components/SiteToc.vue'
import { readAppearance } from './appearance'

const { frontmatter } = useData()
const route = useRoute()

const drawerOpen = ref(false)
const home = computed(() => frontmatter.value.layout === 'home')

onMounted(readAppearance)
watch(
  () => route.path,
  () => {
    drawerOpen.value = false
  },
)
</script>

<template>
  <div class="shell" :class="{ 'shell--home': home }">
    <SiteHeader :drawer-open="drawerOpen" @toggle-drawer="drawerOpen = !drawerOpen" />

    <template v-if="home">
      <main class="home">
        <HomeHero />
        <div class="content content--home">
          <Content />
        </div>
      </main>
    </template>

    <template v-else>
      <div class="page">
        <div class="page__rail" :class="{ 'is-open': drawerOpen }">
          <SiteSidebar />
        </div>
        <div
          v-if="drawerOpen"
          class="page__scrim"
          @click="drawerOpen = false"
        />

        <main class="page__main">
          <article class="content">
            <Content />
          </article>
        </main>

        <SiteToc />
      </div>
    </template>

    <SiteFooter />
  </div>
</template>
