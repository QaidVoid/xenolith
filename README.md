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
| functions lifted to C | 27,432 (99.9%) | 11,728 (98.5%) |
| of those, import thunks | 156 | 187 |
| emitted C compiles | yes, all of it | yes, all of it |

The scalar and vector instruction sets are modelled as far as these two titles
exercise them, including the console's own vector forms. What is left is 190
functions, every one of them stopped by the Direct3D vertex pack or unpack,
whose type field selects a format this project has no independent reading of.

It links, and running it gets as far as the first call into the operating
system. A runtime ships with it that maps guest memory, loads the image, and
implements what the interface declares. What it does not do is service an
import, create a thread, or draw anything, so a title entered at its recorded
entry point reaches something unimplemented and stops there, naming it.

That is the honest description of how far this has got: the translation is
complete enough to link twelve thousand functions and run them, and there is no
environment underneath for them to run in.

Two registers are modelled as storage whose architectural effects are not
honored: masking interrupts does nothing, and a rounding mode written to the
floating point status register does not change how a later operation rounds.
Both are stated in the runtime interface beside their declaration rather than
left to be discovered.

The output is a directory of translation units, the runtime, the decoded image,
and a makefile that links them into a program. The larger title emits 93 units
and takes a few minutes with `make -j`, which is the shape the output has to be
in for a build to use more than one core.

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
  against GNU `objdump` for the values it extracts and the order it prints them
  in. Both agree across every distinct encoding in both titles' code sections,
  942,332 instructions with no disagreement.
- **The analysis** is checked against jump tables and helper addresses worked out
  by hand elsewhere, and asserts that no block is ever claimed without an edge
  reaching it.
- **The instruction model** is checked against 2.14 million instructions another
  project emitted for the same title, comparing which registers each touches.
- **The semantics** are checked by running the instruction on emulated PowerPC
  hardware and running the C this project emits for it, from the same state, and
  comparing the general, floating point, and vector registers, the condition
  fields, the carry, and memory afterwards.
- **Whole functions** out of a shipped title are run the same way, on emulated
  hardware against the real image and through the emitted C on the host, and
  compared. The instruction under test is not one this project picked, and
  neither is the shape it sits in.

The instruction differential found six real mistakes on its first run, including
one where recording a result compared the low half of a register rather than the
whole of it, so every dot suffixed instruction set the wrong condition whenever a
result's halves disagreed in sign. It later found a seventh by not finishing: a
conditional store's low encoding bit was read as a record bit, so a comparison
overwrote the field the store had set, and the retry after it looped forever. It
found two more in the vector families: the fused multiply and add rounds once
between the two operations rather than twice, and the console's own forms take
their third source from the register they write.

Running real functions found nine defects in the model, and two more in the
harness checking it. Three needed the second title and two more needed a deeper
sample, which is the argument for running every oracle over both titles and for
not stopping at the first clean pass.

The first three are all in control flow, which is what a differential over
single instructions cannot reach. A conditional branch
to the link register was emitted as an unconditional return, so every conditional
return in every title returned always. The branch forms that decrement the count
register never decremented it, so every counted loop was infinite. And `addic.`
records its result by virtue of its opcode rather than a record bit, and the
emitter only looked at the bit, so a countdown loop tested a condition its own
subtraction should have set and never saw it change.

The other three are places where an operation that looks like it works on
thirty two bits does not. A word rotate whose mask wraps past the last bit
writes both halves of its register, and an insert whose mask does not wrap
leaves the high half alone, where every rotate emitted here zero extended. And
the carry out of the add and subtract family was read from one comparison, which
is sound for a sum of two terms and not for the three these take.

Sampling deeper found two more: a doubleword insert whose mask was taken to run
to the last bit rather than to wherever the shift left off, and a scalar fused
multiply written as a multiply and then an add, which rounds twice where the
architecture rounds once.

Widening it to follow calls and recovered jump tables found the ninth, and the
worst of them: a conditional trap emitted as an unconditional one. Compilers put
a trap after a division to catch a zero divisor, so the emitted code stopped on
every one of those checks rather than on the fault being checked for. Until that
widening, nothing had ever executed a call, a return, one of the eight register
helpers, or any of the recovered jump tables.

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
./lifted/lifted ./lifted/image.bin 0x82090000
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

- **It cannot play a game.** The runtime that ships with it maps memory and
  links, and implements nothing the console provided. An import is a call naming
  a library and an ordinal, and the runtime reports it and stops.
- **The console's vector extension has no execution oracle.** No assembler
  accepts VMX128 and no emulator implements it, so those instructions are checked
  against an emitted corpus and by reading, and nothing more.
- **Compressed containers using LZX are not supported**, nor are title update
  patches. Both are implemented up to the point where a sample was needed.
- **Most real functions cannot be executed under emulation.** The console runs
  its titles in the mode where an effective address is truncated to 32 bits, and
  the model does the same. `qemu-ppc64` runs in the 64 bit mode and truncates
  nothing, so an address a title formed by sign extending something at or above
  two gigabytes arrives with its top half set, which is most of the image's own
  globals. Mapping those pages where the emulator looks for them would fix it,
  and that address is above the host's own limit. Such a function faults there
  and its run is skipped, so about three quarters of a sample is lost. What does
  run is compared exactly.
- **Between 0.1 and 1.5 percent of functions do not lift**, all of them
  blocked by the Direct3D vertex pack and unpack. Their encoding is readable
  and their type field is not: learning what each format means would mean
  copying it from another implementation, and an unpack built from a guessed
  format produces plausible floats rather than an obvious mistake.
- **The console's vector extension is modelled without an execution oracle.**
  No assembler accepts it and no emulator implements it, so those instructions
  are checked against the emitted corpus and by reading and nothing else. The
  standard extension is executed: sixty six of its instructions are run on
  emulated hardware and compared. Its permute forms are refused outright,
  because a permute built from the wrong bits produces a plausible vector rather
  than an obvious mistake.
- **Two registers round trip without doing anything.** A program that depends on
  interrupts actually being masked, or on a rounding mode actually changing,
  compiles and is wrong.
- **`dcbz` has no oracle.** Its block size is the console's, which a general
  purpose emulator does not share, so it is stated from the documentation rather
  than checked against hardware.

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
