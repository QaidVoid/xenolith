# Environment

## Key material

A key and game data are never tracked in this repository. `.gitignore` excludes
`corpus/`, `tmpwork/`, and files ending in `.key`, `.xex`, `.iso` or `.bin`, and
`scripts/check-secrets` fails the gate when one of them is tracked anyway, or
when a tracked file assigns a value to the key variable, grouped with
underscores or spaces or not. It reports the path and the line, and never the
value.

| variable | what it names |
|---|---|
| `XENOLITH_TRACE_UNLIFTED` | set to report every function a built title called that was never translated, and carry on past it |
| `XENOLITH_TRACE_DISPATCH` | set to report every address a built title dispatched to and could not reach, rather than stopping at the first |
| `XENOLITH_TRACE_IMPORTS` | set to report every import a built title reaches, rather than stopping at the first |
| `XENOLITH_TITLE_SAMPLE` | how many functions the real function differential runs, for a deeper sweep |
| `XENOLITH_XEX_KEY` | the static key, as 32 hexadecimal digits |

Three sources are consulted in order: `--key-file`, this variable, then
`~/.config/xenolith/xex.key`. See [Key material](/guide/keys).

## What the host cannot do

Two things a run does depend on the machine it runs on, and neither is a fault in
a title. Both are reported once, in one line naming the host, and the run carries
on without them.

| variable | what it does |
|---|---|
| `XENOLITH_NO_WINDOW` | set to open no window at all, so a run that is being measured shows nothing |

A host with no way to show a frame prints one line beginning `no frame will be
shown, because`, names what was missing, and says the run carries on. A host
with no sound device prints one line beginning `no sound will be heard:` and does
the same. Neither stops the run, and neither is a translation defect.

## Running the tests for real

No game data is in this repository. The tests that need a real title, a cross
toolchain, or an emulator read paths from the environment and skip when they are
unset, so `cargo test` passes with nothing installed.

That is a weaker result than it looks. To run the checks that mean something:

| variable | what it names |
|---|---|
| `XENOLITH_TEST_XEX` | a container to parse |
| `XENOLITH_TEST_IMAGE` | a reference image from another implementation, to compare decoding against |
| `XENOLITH_ANALYSIS_XEX` | a container to analyze |
| `XENOLITH_ANALYSIS_HELPERS` | expected helper addresses, `0xADDR` or `0xADDR@16` |
| `XENOLITH_ANALYSIS_SWITCHES` | a reference jump table file to compare against |
| `XENOLITH_ANALYSIS_TABLE` | one hand worked table, to pin the recovery |
| `XENOLITH_LIFT_IMAGE` | the container an emitted corpus came from |
| `XENOLITH_LIFT_CORPUS` | a directory of emitted C++ to compare semantics against |
| `XENOLITH_SHADER_IMAGE` | a title's image, to find the shaders a title handed over among |
| `XENOLITH_HOLD` | a controller number to report as held, for a run with no window |
| `XENOLITH_TRACE_SOUND` | set to report how far playback has got |
| `XENOLITH_PPC_BINUTILS` | a cross binutils prefix, for the operand comparison |
| `XENOLITH_PPC_CODE` | a code section to compare operands over |
| `XENOLITH_PPC_TOOLCHAIN` | a big endian ppc64 toolchain prefix |
| `XENOLITH_PPC_EMULATOR` | a user mode emulator, `qemu-ppc64` by default |

Run these with `--release`. In a debug build they take minutes.

```sh
XENOLITH_PPC_TOOLCHAIN=/opt/bootlin/powerpc64-power8-glibc/bin/powerpc64-buildroot-linux-gnu- \
XENOLITH_PPC_EMULATOR=qemu-ppc64 \
  cargo test --release -p xenolith-lift --test execution
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
