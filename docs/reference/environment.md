# Environment

Every environment variable the tool, the runtime and the test suite read.
`crates/xenolith/tests/environment_page.rs` fails the gate when this page and
the code disagree in either direction.

## Key material

The static key is looked for in this order: `--key-file`, then
`XENOLITH_XEX_KEY`, then `xenolith/xex.key` under the configuration directory.
`inspect` reads only headers and needs no key; `analyze`, `disasm` and `lift`
decode the image and do.

| variable | what it names |
|---|---|
| `XENOLITH_XEX_KEY` | the static key, as 32 hexadecimal digits |
| `XDG_CONFIG_HOME` | the configuration directory holding `xenolith/xex.key` |
| `HOME` | used for `~/.config` when `XDG_CONFIG_HOME` is not set |

A key and game data are never tracked in the repository. `.gitignore` excludes
`corpus/`, `tmpwork/` and files ending in `.key`, `.xex`, `.iso` or `.bin`, and
`scripts/check-secrets` fails the gate if one is tracked anyway or a tracked file
assigns the key variable. It reports the path and line, never the value.

## Running a lifted title

| variable | what it does |
|---|---|
| `DISPLAY` | the X11 display to open a window on |
| `WAYLAND_DISPLAY` | the Wayland desktop to open a window on. Tried before `DISPLAY`, which is used instead when the drawing device cannot present to the Wayland desktop |
| `XENOLITH_NO_WINDOW` | open no window and draw off screen |
| `XENOLITH_HOLD` | buttons to report as held, for a run without a window: `0x10@40-41` holds `0x10` from second 40 to 41, `0x10@9000f-9010f` from shown frame 9000 to 9010, and several are separated by commas |
| `XENOLITH_SCALING` | `nearest` scales frames to the window without filtering; the default is linear |
| `XENOLITH_GAME` | the folder holding the player's copy of the game, `default.xex` among its files, in place of `games/<id>/` in xenolith's directory |
| `XENOLITH_DATA` | xenolith's own directory of the user's data, a whole path. Games are looked for in `games/<id>/` under it and saves go in `saves/<id>/` |
| `XDG_DATA_HOME` | where the user's data is kept, used where `XENOLITH_DATA` is not set. xenolith's directory is `xenolith/` under it, or `~/.local/share/xenolith` where neither is set |

A host without a window, a sound device or libavcodec prints one line saying
so, beginning `no frame will be shown, because`, `no sound will be heard:` or
`compressed sound is not decoded:`, and the run carries on.

## Tracing

Each of these makes a run report something and carry on.

| variable | what it does |
|---|---|
| `XENOLITH_TRACE_UNLIFTED` | report every call into a function that was not lifted, and return from it |
| `XENOLITH_TRACE_DISPATCH` | report every indirect branch to an address no function answers |
| `XENOLITH_TRACE_IMPORTS` | report every import the title calls, answering each with nothing |
| `XENOLITH_TRACE_SOUND` | report how much sound has been played and how much of it was silence |
| `XENOLITH_TRACE_DRAWS` | describe the draws a title makes, once per shader |
| `XENOLITH_CORNERS_AT` | with `XENOLITH_TRACE_DRAWS`, describe every draw of the frame with this number instead |

`XENOLITH_TRACE_UNLIFTED` and `XENOLITH_TRACE_IMPORTS` change what the title is
told, so a run with either one on is a diagnostic rather than a faithful run.

## Keeping what a run sees

| variable | what it does |
|---|---|
| `XENOLITH_FRAMES` | a directory; every frame shown is written there as `frameNNNN.ppm` |
| `XENOLITH_KEEP_PICTURES` | a directory; textures stored a byte per channel are written there |
| `XENOLITH_KEEP_SHADERS` | a directory; the microcode a title hands over and the SPIR-V it becomes are written there |
| `XENOLITH_KEEP_PACKETS` | print the words of the GPU command packets the runtime reads |
| `XENOLITH_DUMP_FAILED_SHADER` | print the words of a shader that would not translate |

## Isolating a drawing problem

| variable | what it does |
|---|---|
| `XENOLITH_ONLY_SHADER` | a vertex shader hash in hexadecimal; only draws using it are drawn |
| `XENOLITH_PAINT_EVERYTHING` | paint every fragment one colour, to see whether a shape reaches the surface at all |

## The checks that need real data

No game data is in the repository. Tests that need a title, a cross toolchain
or an emulator read their inputs from these and are skipped without them, so
`cargo test` passes with nothing installed. `scripts/check-oracles` runs them
and treats a skipped test as a failure.

| variable | what it names |
|---|---|
| `XENOLITH_TEST_XEX` | a container to parse |
| `XENOLITH_TEST_IMAGE` | a reference image from another implementation, to compare decoding against |
| `XENOLITH_ANALYSIS_XEX` | a container to analyse |
| `XENOLITH_ANALYSIS_IMAGE` | a decoded image to analyse |
| `XENOLITH_ANALYSIS_BASE` | the address that image loads at |
| `XENOLITH_ANALYSIS_SECTIONS` | its section table, as `start:size:kind` |
| `XENOLITH_ANALYSIS_HELPERS` | expected helper addresses, `0xADDR` or `0xADDR@16` |
| `XENOLITH_ANALYSIS_SWITCHES` | a reference jump table file to compare against |
| `XENOLITH_ANALYSIS_TABLE` | one hand worked table, to pin the recovery |
| `XENOLITH_LIFT_IMAGE` | the container an emitted corpus came from |
| `XENOLITH_LIFT_CORPUS` | a directory of emitted C++ to compare register use against |
| `XENOLITH_SHADER_IMAGE` | a title's image, to find shaders in |
| `XENOLITH_TITLE_SAMPLE` | how many functions the real function comparison runs |
| `XENOLITH_PPC_BINUTILS` | a cross binutils prefix, for the operand comparison |
| `XENOLITH_PPC_CODE` | a code section to compare operands over |
| `XENOLITH_PPC_TOOLCHAIN` | a big endian ppc64 toolchain prefix |
| `XENOLITH_PPC_EMULATOR` | a user mode emulator, `qemu-ppc64` by default |
| `XENOLITH_PPC32_TOOLCHAIN` | a big endian 32 bit ppc toolchain prefix, for the condition and carry results a console computes |
| `XENOLITH_PPC32_EMULATOR` | a 32 bit user mode emulator, `qemu-ppc` by default |

Run these with `--release`; a debug build takes minutes.

```sh
XENOLITH_PPC_TOOLCHAIN=/opt/bootlin/powerpc64-power8-glibc/bin/powerpc64-buildroot-linux-gnu- \
XENOLITH_PPC_EMULATOR=qemu-ppc64 \
XENOLITH_PPC32_TOOLCHAIN=powerpc-linux-gnu- \
  cargo test --release -p xenolith-lift --test execution -- --include-ignored
```

`XENOLITH_LIFT_IMAGE` is separate from `XENOLITH_ANALYSIS_XEX` on purpose. A
corpus compared against the wrong title produces thousands of differences that
look like model bugs, so the test refuses when the two disagree.

### Getting the tools

The execution comparison needs a big endian ppc64 cross toolchain, such as a
Bootlin `powerpc64-power8-glibc` toolchain, and `qemu-ppc64` from a distribution
package. Big endian matters: the console is a big endian PowerPC, and a little
endian toolchain would build programs whose memory layout disagrees with the
model. `scripts/check` runs the comparison whenever both are installed.
