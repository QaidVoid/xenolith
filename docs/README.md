# Documentation

The site is VitePress with a theme written here rather than extended from the
default one, since the default is built for a library's API reference and this
is not that.

```sh
bun install
bun run dev      # serve at localhost:5173 with reload
bun run build    # render into .vitepress/dist
bun run preview  # serve what was rendered
```

## Where things are

| path | what it holds |
|---|---|
| `.vitepress/config.mts` | site metadata, and the sidebar, which is written out rather than generated |
| `.vitepress/theme/` | the layout, its components, and the stylesheet |
| `guide/` | using the tool |
| `internals/` | how each stage works |
| `verification.md` | what each stage is checked against |
| `reference/` | commands and environment variables |

## Writing here

Two rules, both carried over from the rest of the project.

**ASCII only.** Use HTML entities in templates where a symbol is wanted.

**Say what is true, including what is missing.** Every number on these pages
comes from a run against a real title, and every claim about what the tool
cannot do is there because it cannot do it. A documentation site that oversells
a recompiler wastes the reader's afternoon before they find out.
