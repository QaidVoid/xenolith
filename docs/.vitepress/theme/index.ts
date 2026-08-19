import type { Theme } from 'vitepress'

import Layout from './Layout.vue'
import './style.css'

/**
 * A theme written here rather than extended from the default one.
 *
 * The default theme is built for a library's API documentation. This site is
 * about a program that reads machine code, and the layout leans on that: the
 * navigation reads like a memory map and the landing page shows the two sides
 * of the translation next to each other.
 */
export default { Layout } satisfies Theme
