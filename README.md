# xenolith

A static recompiler for Xbox 360 executables. It reads a XEX container, decodes
the PowerPC code inside it, works out where the functions are, and writes C.

Linux only, and deliberately so. There is no Windows support and none planned.

Fuller documentation lives in `docs/`, covering the pipeline stage by stage, the
oracles each stage is checked against, and every subcommand. Build it with
`cd docs && bun install && bun run dev`.

## Status

This turns a retail game into C that compiles. It does not yet turn one into a
game you can play.

Measured against two retail titles:

| | title A | title B |
|---|---|---|
| functions discovered | 27,447 | 11,903 |
| executable words claimed | 95.3% | 86.9% |
| functions lifted to C | 26,908 (98.0%) | 11,195 (94.1%) |
| of those, import thunks | 156 | 187 |
| emitted C compiles | yes, all of it | yes, all of it |

What that leaves out is the part that matters for running anything. The emitted C
is written against a runtime interface this project declares and does not
implement: no guest memory is mapped, no import does anything, no threads exist.
Around 1,460 addresses are declared without a definition, mostly functions that
could not be lifted plus the register save and restore helpers. So it compiles
and it does not link, which is an honest description of how far this has got.

The output is a directory of translation units and a makefile that builds them.
The larger title emits 88 units and takes two and a half minutes to compile with
`make -j`, which is the shape the output has to be in for a build to use more
than one core.

## What is here

Four crates, each usable on its own:

- `xenolith-xex` reads the container: headers, page descriptors, imports and
  what each one names, decryption, and the decompression schemes.
- `xenolith-ppc` decodes the instruction set: 440 instructions covering 64 bit
  PowerPC, AltiVec, and the console's VMX128 extension.
- `xenolith-analysis` finds functions, basic blocks, and control flow edges,
  detects the register save and restore helpers by their shape, and recovers the
  jump tables behind indirect branches.
- `xenolith-lift` describes what each instruction reads, writes, and computes,
  and emits C for a whole function.

The `xenolith` binary exposes them as `inspect`, `disasm`, `analyze`, and `lift`.

## The point of it

An existing tool does this job and requires, per game, a hand written file naming
every function boundary, the addresses of eight register helpers, setjmp and
longjmp, and byte patterns to skip. Its companion emits a separate file of jump
tables, which for one title runs to 852 entries across 30,250 lines.

This project accepts no per title configuration at all. Everything above is
recovered, or reported as unrecovered. Nothing is guessed:

- All eight register helper addresses are detected on both titles without being
  given any of them, matched against values two other projects recorded by hand.
- 803 of the 852 jump tables are recovered and agree exactly with what that tool
  produced, with zero disagreements.
- Every import record is read from the image and reported as a library and an
  ordinal, so the emitted code names each call into the operating system rather
  than leaving it among the functions that failed.
- An instruction the decoder does not recognize reports itself unknown, and a
  function holding an instruction the model cannot express is not emitted at all.

## How it is checked

Nothing here is trusted because it looks right. Every layer is compared against
something produced independently:

- **The container** is checked against two retail titles, with the arithmetic of
  the header offsets confirmed rather than assumed.
- **The decoder** is checked against `llvm-mc` for the instruction it names, and
  against GNU `objdump` for the values it extracts. Operand values agree across
  every distinct encoding in both titles' code sections.
- **The analysis** is checked against jump tables and helper addresses worked out
  by hand elsewhere, and asserts that no block is ever claimed without an edge
  reaching it.
- **The instruction model** is checked against 1.19 million instructions another
  project emitted for the same title, comparing which registers each touches.
- **The semantics** are checked by running the instruction on emulated PowerPC
  hardware and running the C this project emits for it, from the same state, and
  comparing the registers, condition fields, carry, and memory afterwards.

That last one is the strongest and the newest. On its first run it found six real
mistakes, including one where recording a result compared the low half of a
register rather than the whole of it, so every dot suffixed instruction set the
wrong condition whenever a result's halves disagreed in sign.

Fuzz targets cover the loader, the decoder, the analysis, and the lifter.

## Building

```
cargo build --release
cargo test
```

Nothing beyond a Rust toolchain is needed. Rust 1.85 or later.

## Running it

```
xenolith inspect  default.xex
xenolith inspect  default.xex --imports
xenolith disasm   default.xex --start 0x82090000 --length 256
xenolith analyze  default.xex
xenolith lift     default.xex --out ./lifted --blockers
make -C ./lifted -j
```

A retail XEX is encrypted, and this project ships no key. Supply one through
`XENOLITH_XEX_KEY` as 32 hexadecimal digits, or write it to
`~/.config/xenolith/xex.key`. Every subcommand also accepts `--raw` to read an
image something else already decoded, which needs no key at all.

## Testing against a real title

No game data is in this repository. The tests that need a real title read it from
the environment and skip when it is absent, so `cargo test` passes with nothing
installed. To run them for real:

| variable | what it names |
|---|---|
| `XENOLITH_XEX_KEY` | the static key, as 32 hexadecimal digits |
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

Run those with `--release`. In a debug build they take minutes.

## What it cannot do

Stated plainly, because a coverage figure implies more than it means:

- **It cannot run anything.** The runtime interface is a declaration. An import
  is emitted as a call naming a library and an ordinal, and nothing answers it.
- **The console's vector extension has no execution oracle.** No assembler
  accepts VMX128 and no emulator implements it, so those instructions are checked
  against an emitted corpus and by reading, and nothing more.
- **Compressed containers using LZX are not supported**, nor are title update
  patches. Both are implemented up to the point where a sample was needed.
- **Between 2 and 6 percent of functions do not lift**, blocked by instructions
  the model does not yet describe, most of them vector ones. The `lift`
  subcommand ranks them by how many functions each blocks, which is the list to
  work from.

## Licence

Dual licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you state otherwise, any contribution you intentionally submit for
inclusion in this work shall be dual licensed as above, without any additional
terms or conditions.

The instruction table is written from published architecture documentation and
the public record for the console's extension. It derives from no existing
decoder.
