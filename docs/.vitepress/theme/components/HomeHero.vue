<script setup lang="ts">
import { useData } from 'vitepress'

const { theme } = useData()

/**
 * A real fragment of one title's output, disassembly beside the C emitted for
 * it. Made up code would show the layout and say nothing about the work.
 */
const rows = [
  ['0x82090000', 'mfspr r12, r8, r0', 'ctx->r[12] = ctx->lr;'],
  ['0x82090004', 'stw r12, -8(r1)', 'xenolith_store32(base, address, ...);'],
  ['0x82090010', 'lis r11, -32251', 'ctx->r[11] = (uint64_t)(int64_t)(...);'],
  ['0x8209001c', 'rlwinm r10, r4, 0, 31, 31', 'ctx->r[10] = (uint32_t)ctx->r[4] & 1u;'],
  ['0x82090020', 'cmpli cr6, 0, r10, 0', 'ctx->cr[6].eq = ... == (0ull);'],
  ['0x82090028', 'bc 12, 26 0x82090034', 'if (ctx->cr[6].eq) { goto loc_82090034; }'],
]

const figures = [
  { value: '440', label: 'instructions decoded' },
  { value: '39,350', label: 'functions discovered' },
  { value: '98.0%', label: 'lifted on the larger title' },
  { value: '0', label: 'lines of per title configuration' },
]
</script>

<template>
  <section class="hero">
    <div class="hero__grid" aria-hidden="true" />

    <div class="hero__body">
      <p class="hero__eyebrow">
        <span class="hero__dot" />
        static recompiler &middot; linux only &middot; clean room
      </p>

      <h1 class="hero__title">
        Xbox 360 machine code<br />
        <span class="hero__accent">becomes C you can read.</span>
      </h1>

      <p class="hero__lede">
        xenolith reads a XEX container, decodes the PowerPC inside it, works out
        where the functions are, and writes C. It takes no per title
        configuration at all. Everything another tool would ask you to write by
        hand is recovered, or reported as unrecovered.
      </p>

      <div class="hero__actions">
        <a class="button button--solid" href="/guide/getting-started">
          Get started
        </a>
        <a class="button" :href="theme.repository" rel="noreferrer">Source</a>
      </div>
    </div>

    <div class="hero__panel">
      <div class="panel">
        <div class="panel__chrome">
          <span class="panel__side">powerpc</span>
          <span class="panel__arrow">&rarr;</span>
          <span class="panel__side">c</span>
        </div>
        <table class="panel__table">
          <tbody>
            <tr v-for="row in rows" :key="row[0]">
              <td class="panel__address">{{ row[0] }}</td>
              <td class="panel__asm">{{ row[1] }}</td>
              <td class="panel__c">{{ row[2] }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <ul class="figures">
      <li v-for="figure in figures" :key="figure.label">
        <strong>{{ figure.value }}</strong>
        <span>{{ figure.label }}</span>
      </li>
    </ul>
  </section>
</template>
