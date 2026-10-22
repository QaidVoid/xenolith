import { defineConfig } from 'vitepress'

/**
 * The sidebar is written out rather than generated from the directory, because
 * the order pages should be read in is not the order a filesystem lists them.
 */
const sections = [
  {
    label: 'Start here',
    address: '0x00',
    items: [
      { text: 'What this is', link: '/guide/' },
      { text: 'Getting started', link: '/guide/getting-started' },
      { text: 'Key material', link: '/guide/keys' },
      { text: 'Lifting a title', link: '/guide/lifting' },
    ],
  },
  {
    label: 'How it works',
    address: '0x10',
    items: [
      { text: 'The container', link: '/internals/container' },
      { text: 'The decoder', link: '/internals/decoder' },
      { text: 'Finding functions', link: '/internals/analysis' },
      { text: 'Emitting C', link: '/internals/lifting' },
      { text: 'Imports', link: '/internals/imports' },
      { text: 'Reading a shader', link: '/internals/shader' },
      { text: 'The runtime', link: '/internals/runtime' },
    ],
  },
  {
    label: 'How it is checked',
    address: '0x20',
    items: [
      { text: 'Differential testing', link: '/verification/' },
      { text: 'The container', link: '/verification/container' },
      { text: 'The decoder', link: '/verification/decoder' },
      { text: 'Finding functions', link: '/verification/functions' },
      { text: 'The model', link: '/verification/model' },
      { text: 'The runtime', link: '/verification/runtime' },
    ],
  },
  {
    label: 'Reference',
    address: '0x30',
    items: [
      { text: 'Commands', link: '/reference/cli' },
      { text: 'Environment', link: '/reference/environment' },
      { text: 'Output', link: '/reference/output' },
      { text: 'Diagnostics', link: '/reference/errors' },
      { text: 'The runtime interface', link: '/reference/interface' },
    ],
  },
]

export default defineConfig({
  title: 'xenolith',
  description:
    'A static recompiler that turns Xbox 360 executables into C, with no per title configuration.',
  lang: 'en',
  cleanUrls: true,
  appearance: true,
  lastUpdated: false,

  /**
   * The README describes the site to whoever maintains it, so it is not a page
   * a reader reaches and has no place in the sidebar.
   */
  srcExclude: ['README.md'],

  head: [
    ['meta', { name: 'color-scheme', content: 'dark light' }],
    [
      'link',
      {
        rel: 'icon',
        href:
          'data:image/svg+xml,' +
          encodeURIComponent(
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">' +
              '<rect width="32" height="32" rx="6" fill="#12100e"/>' +
              '<path d="M8 9l16 14M24 9L8 23" stroke="#e0913a" stroke-width="3" ' +
              'stroke-linecap="round"/></svg>',
          ),
      },
    ],
  ],

  markdown: {
    theme: { light: 'github-light', dark: 'vitesse-dark' },
    lineNumbers: false,
  },

  themeConfig: {
    repository: 'https://github.com/QaidVoid/xenolith',
    sections,
  },
})
