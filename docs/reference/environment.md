# Environment

Every variable this tool reads, what it changes, and what happens when it is not
set. A variable the code reads and this page does not name is one nobody can
find, and `crates/xenolith/tests/environment_page.rs` fails the gate when the two
disagree in either direction.

## Key material

A key and game data are never tracked in this repository. `.gitignore` excludes
`corpus/`, `tmpwork/`, and files ending in `.key`, `.xex`, `.iso` or `.bin`, and
`scripts/check-secrets` fails the gate when one of them is tracked anyway, or
when a tracked file assigns a value to the key variable. It reports the path and
the line, and never the value.

Three sources are consulted in order: `--key-file`, then `XENOLITH_XEX_KEY`,
then `~/.config/xenolith/xex.key`. `XDG_CONFIG_HOME` moves the last of those, and
`HOME` is what it falls back to.

`inspect` reads the header and needs no key. `analyze`, `disasm` and `lift`
decode the image and do.

| variable | what it names |
|---|---|
| `XENOLITH_XEX_KEY` | the static key, as 32 hexadecimal digits |
| `XDG_CONFIG_HOME` | where to look for `xenolith/xex.key` when the variable above is not set |
| `HOME` | what that falls back to when `XDG_CONFIG_HOME` is not set |

## Traces

Each of these makes a run report and carry on rather than stop. Each is a
diagnostic, and each makes the run after it describe the answer it was given
rather than the title.

| variable | what it does |
|---|---|
| `XENOLITH_TRACE_UNLIFTED` | report every function a built title called that was never translated |
| `XENOLITH_TRACE_DISPATCH` | report every address a built title dispatched to that no function answers |
| `XENOLITH_TRACE_IMPORTS` | report every import a built title reaches, answering each with nothing |
| `XENOLITH_TRACE_SOUND` | report how far playback has got |
| `XENOLITH_TRACE_DRAWS` | report each draw a title asks for |

## What the runtime keeps

A run throws away almost everything a title hands it, because keeping it is not
needed to draw. These ask for it to be kept, for reading afterwards.

| variable | what it does |
|---|---|
| `XENOLITH_KEEP_PACKETS` | keep the command packets a title wrote |
| `XENOLITH_KEEP_PICTURES` | keep the pictures a title loaded |
| `XENOLITH_KEEP_SHADERS` | keep the shaders a title handed over |
| `XENOLITH_FRAMES` | how many frames to keep |
| `XENOLITH_PAINT_EVERYTHING` | draw every frame rather than only the ones a title asks to show |

## What the renderer assumes

These change what the renderer does with what a title asked for. They exist so
that a question about a title can be asked of a run, not because a console had
them.

| variable | what it does |
|---|---|
| `XENOLITH_CORNERS_AT` | where a triangle's corners are taken from |
| `XENOLITH_SCALING` | how a picture is scaled into the surface |
| `XENOLITH_ONLY_SHADER` | draw with only the shader named, for isolating one |
| `XENOLITH_DUMP_FAILED_SHADER` | write out a shader that would not translate |

## What the host cannot do

Two things a run does depend on the machine it runs on, and neither is a fault in
a title. Both are reported once, in one line naming the host, and the run carries
on without them.

| variable | what it does |
|---|---|
| `XENOLITH_NO_WINDOW` | open no window at all, so a run that is being measured shows nothing |
| `DISPLAY` | the display to open a window on, where the host speaks X11 |
| `WAYLAND_DISPLAY` | the desktop to open a window on, where the host speaks Wayland. This is tried before `DISPLAY`, which is used instead where the device drawing will not draw into the desktop's window, as on a machine whose desktop runs on another device |
| `XENOLITH_HOLD` | a controller number to report as held, for a run with no window; `0x10@40-41` holds it from the fortieth second to the forty first, `0x10@9000f-9010f` from the nine thousandth frame shown to the nine thousand and tenth, and several are given apart by commas |

A host with no way to show a frame prints one line beginning `no frame will be
shown, because`, names what was missing, and says the run carries on. A host with
no sound device prints one line beginning `no sound will be heard:` and does the
same, and one without libavcodec prints one line beginning `compressed sound is
not decoded:`. None of them stops the run, and none is a translation defect.

## Running the checks for real

No game data is in this repository. The tests that need a real title, a cross
toolchain, or an emulator read paths from these and skip when they are unset, so
`cargo test` passes with nothing installed.

| variable | what it names |
|---|---|
| `XENOLITH_TEST_XEX` | a container to parse |
| `XENOLITH_TEST_IMAGE` | a reference image from another implementation, to compare decoding against |
| `XENOLITH_ANALYSIS_XEX` | a container to analyze |
| `XENOLITH_ANALYSIS_IMAGE` | a decoded image to analyze |
| `XENOLITH_ANALYSIS_BASE` | the address that image loads at |
| `XENOLITH_ANALYSIS_SECTIONS` | its section table, as `start:size:kind` |
| `XENOLITH_ANALYSIS_HELPERS` | expected helper addresses, `0xADDR` or `0xADDR@16` |
| `XENOLITH_ANALYSIS_SWITCHES` | a reference jump table file to compare against |
| `XENOLITH_ANALYSIS_TABLE` | one hand worked table, to pin the recovery |
| `XENOLITH_LIFT_IMAGE` | the container an emitted corpus came from |
| `XENOLITH_LIFT_CORPUS` | a directory of emitted C++ to compare semantics against |
| `XENOLITH_SHADER_IMAGE` | a title's image, to find the shaders a title handed over among |
| `XENOLITH_TITLE_SAMPLE` | how many functions the real function differential runs, for a deeper sweep |
| `XENOLITH_PPC_BINUTILS` | a cross binutils prefix, for the operand comparison |
| `XENOLITH_PPC_CODE` | a code section to compare operands over |
| `XENOLITH_PPC_TOOLCHAIN` | a big endian ppc64 toolchain prefix |
| `XENOLITH_PPC_EMULATOR` | a user mode emulator, `qemu-ppc64` by default |
| `XENOLITH_PPC32_TOOLCHAIN` | a big endian thirty two bit ppc toolchain prefix, for the condition and the carry a console computes |
| `XENOLITH_PPC32_EMULATOR` | a thirty two bit user mode emulator, `qemu-ppc` by default |

Run these with `--release`. In a debug build they take minutes.

```sh
XENOLITH_PPC_TOOLCHAIN=/opt/bootlin/powerpc64-power8-glibc/bin/powerpc64-buildroot-linux-gnu- \
XENOLITH_PPC_EMULATOR=qemu-ppc64 \
XENOLITH_PPC32_TOOLCHAIN=powerpc-linux-gnu- \
  cargo test --release -p xenolith-lift --test execution -- --include-ignored
```

`XENOLITH_LIFT_IMAGE` is kept separate from `XENOLITH_ANALYSIS_XEX` on purpose.
A corpus compared against the wrong title produces thousands of differences that
look like model bugs, so the test refuses when the two disagree rather than
reporting them.

## Getting the tools

The execution differential needs a big endian ppc64 cross toolchain and a user
mode emulator. A prebuilt toolchain works; the one used here is a Bootlin
`powerpc64-power8-glibc` toolchain, and the emulator is `qemu-ppc64` from a
distribution package.

Big endian matters. The console is a big endian PowerPC, and a little endian
toolchain would assemble the same mnemonics into a program whose memory layout
disagrees with everything the model does.

`scripts/check` runs the differential when both are installed and says so when
they are not, so installing them and never running them is not a state the gate
can be in.
