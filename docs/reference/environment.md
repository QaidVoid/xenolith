# Environment

## Key material

| variable | what it names |
|---|---|
| `XENOLITH_TRACE_DISPATCH` | set to report every address a built title dispatched to and could not reach, rather than stopping at the first |
| `XENOLITH_TRACE_IMPORTS` | set to report every import a built title reaches, rather than stopping at the first |
| `XENOLITH_TITLE_SAMPLE` | how many functions the real function differential runs, for a deeper sweep |
| `XENOLITH_XEX_KEY` | the static key, as 32 hexadecimal digits |

Three sources are consulted in order: `--key-file`, this variable, then
`~/.config/xenolith/xex.key`. See [Key material](/guide/keys).

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
