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
| functions discovered | 32,738 | 14,558 |
| executable words claimed | 97.7% | 92.8% |
| functions lifted to C | 32,727 (99.97%) | 14,548 (99.93%) |
| of those, import thunks | 156 | 188 |
| emitted C compiles | yes, all of it | yes, all of it |
| helper entries emitted beside them | 50 | 60 |
| runs its whole startup | yes | yes |

The scalar and vector instruction sets are modelled as far as these two titles
exercise them, including the console's own vector forms. What is left is 21
functions across both titles, every one of them stopped by a Direct3D vertex
pack or unpack whose type field selects a format this project has no reading of.

It links, and running it from a title's recorded entry point gets as far as the
first call into the operating system, on both titles. Getting there needed the
register save and restore helpers: a call into one lands partway through it, and
discovery does not claim those addresses, so they were emitted as traps. Better
than a third of a title's functions save registers that way, which meant the
program stopped on the first thing it did. Each entry a caller uses is now
lifted from where the caller enters it.

A runtime ships with it that maps guest memory, loads the image, gives the guest
a stack, and answers the nine kernel entry points a title's startup reaches:
reserving and committing memory, the process type, a critical section, thread
local storage, and the privilege check. Each rests on a documented shape or on
this runtime having one thread, and the two that rest on neither say so the
first time they answer.

Both titles run their whole startup. Handed the table of static constructors a
title calls through, one runs all 217 of them and stops at a kernel call it had
never got far enough to make.

Those constructors have to be handed over, and the reason is worth stating.
They are functions with no prologue, named only from a run of pointers. Claiming
every unclaimed address a word in the image names would find them and 10,269
coincidences besides, since any aligned word in range looks like a pointer to
code. Claiming every entry of every run of such words does no better: of those,
53 percent are function starts, a fifth point inside a function the way a jump
table entry does, and claiming one of those would split the function it lands
in.

So the evidence used is that a program reached the address. Running a built
title with `XENOLITH_TRACE_DISPATCH` reports every address it wanted and could
not reach, and `lift --roots-from` takes that back. The two alternate until a
round finds nothing new, which for one title is three rounds: two addresses,
then a hundred and twenty, then none.

At the end of that a title lifts around 14,700 functions, runs every static
constructor it has, and stops inside a function that uses the Direct3D vertex
unpack. The startup path is finished; what stops it now is the coverage gap
named above.

Walked past that gap with `XENOLITH_TRACE_UNLIFTED`, a title asks for 23 entry
points, and only one of them is graphics: the video driver being told where the
command buffer identifier lives, handed the address the console keeps its
graphics registers at. The rest are threads and what threads need. A title
creates eight of them, with events, mutants, semaphores and thirteen physical
allocations, before it says anything to the video driver at all.

So threads are what the runtime does next, and they are real ones. A guest
thread runs on a host thread with its own register state and its own stack, the
objects a title waits on are held behind handles, and a critical section is a
lock kept beside the guest address it lives at. The reservation the emitted code
uses stopped being safe the moment a second thread existed, and is now a compare
and swap under a lock, which is stated beside it because it is not quite what
the hardware does.

With that serviced rather than faked, what a title still asks for falls from 23
entry points to five, and then to none. Networking reports that there is none,
since saying it started would send a title looking for one. A module lookup
reports that nothing is loaded here but the title. A time is taken apart by the
documented arithmetic, a calendar being a definition rather than a property of
the hardware. And the system version is read out of the title: the code asking
for it holds `0x200a3200` and goes elsewhere when given less, so what it will
accept is stated in the artefact rather than assumed.

Every entry point the title reaches is now answered, and the vertex unpack that
stopped it in both places is modelled for the form the titles actually use. That
form is 244 of the 281 unpack sites across the two of them: a pair of signed
halfwords, each added into the mantissa of three so the result arrives as three
plus the halfword over four million, which the caller then subtracts and scales.
The type field's meaning was read from XenonRecomp rather than derived here, so
it was checked before it was trusted: that project stores a vector in the
opposite byte order, and the translation between the two was pinned against the
one unpack form this project had already proven on its own, the colour form,
where it reproduces the lane mapping exactly. Both titles carry a vector of
threes in their constant pools, which is what the caller subtracts.

Modelling it took one title from 14,284 functions lifted to 14,548, and the
functions blocked across both from 208 to 21. Nothing is now unlifted on the
path either title runs.

What stopped the run instead was a call through a null function pointer, and
what that turned out to mean is the worst thing found here so far. No guest
thread had ever executed an instruction.

A title does not start a thread at the function it wants to run. It names a
shim and hands the shim the real entry, so every thread in the title begins at
the same address, and discovery never claims that address because nothing in the
image branches to it. The runtime looked it up, found nothing, skipped the call
and let the thread finish. A thread that runs nothing and exits looks exactly
like a thread that ran and returned, so eight threads started, eight threads
exited, and the run carried on looking healthy. The silence was the defect.

The symptom surfaced about a thousand functions away, in a subsystem whose
constructor starts a worker and expects that worker to install a delegate. The
constructor ran. The worker was created. The delegate was never installed. Every
step between those facts looked correct.

A thread whose start was never lifted is now reported, on the same footing as an
indirect branch to an address no function answers to, and through the same path
so it prints the same line. That is what makes it self correcting: the loop that
already recovers static constructors takes the shim back on its own, with no new
mechanism and nothing per title. Guest threads then run, and a title reaches a
kernel entry point it had never got far enough to ask for, releasing one object
while waiting on another.

What stopped it after that was a title's own heap refusing an eleven megabyte
allocation during video initialisation, four calls after it asks for a 1280 by
720 display mode, and the cause was in the runtime rather than the title. The
console has two allocators and this runtime had been answering both with one.
They do not have the same shape: the virtual one takes an address and a size
through pointers and returns a status, and the physical one takes its size by
value and returns the address itself. Read the second as though it were the
first, a flag word is taken for a pointer, the answer is written over whatever
that addresses, and the title is handed nothing. Every physical allocation a
title made had failed, silently, and it was asking for the memory it was going
to draw into.

With each read the way it is documented, a title gets its eleven megabytes and
goes on into the graphics driver. That is where it stops now, on the call that
starts the engines, and the entry points beyond it are the ring buffer, the
display mode, the interrupt callback and the rest of the same family. Nothing
here talks to graphics hardware yet, so that is the next body of work rather
than the next bug.

A built title can also be asked what it needs:
`XENOLITH_TRACE_IMPORTS=1` reports each import a run reaches, with the registers
its arguments are in, and carries on as though it had returned nothing. Both
titles ask for the same nine kernel entry points in the same order before they
go anywhere else, which is the C runtime starting up and is not per title.

That trace is a diagnostic and not an environment. A title told every import
returned zero believes it, so the run after the first one is a list of what was
wanted rather than a title running. What it does not do is create a thread or draw
anything, and it answers nothing beyond what a title has actually reached.

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
and takes a few minutes with `make -j8`, which is the shape the output has to be
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
- 822 of the 852 jump tables are recovered and agree exactly with what that tool
  produced, with zero disagreements, and more are found that it does not have.
  What is left is branches whose tables were not read and branches that have no
  table at all, being calls through a pointer held in an object.
- Every import record is read from the image and reported as a library and an
  ordinal, so the emitted code names each call into the operating system rather
  than leaving it among the functions that failed.
- An instruction the decoder does not recognize reports itself unknown, and a
  function holding an instruction the model cannot express is not emitted at all.
- Every address the image points at that begins like a function is claimed by
  one, and so is every trampoline it names, being one branch a linker left where
  a call could not reach. What is left unclaimed is data, jump table entries, and leaf functions
  that leave no prologue to recognize.

## How it is checked

Nothing here is trusted because it looks right. Every layer is compared against
something produced independently:

- **The container** is checked against two retail titles, with the arithmetic of
  the header offsets confirmed rather than assumed.
- **The decoder** is checked against `llvm-mc` for the instruction it names,
  over every instruction the table declares with generated operands, and against
  GNU `objdump` for the values it extracts and the order it prints them in. The
  objdump comparison reads the first twenty thousand distinct encodings of each
  title and compares about thirteen thousand of them each way, with no
  disagreement in either. That is a sample and not the whole of either title.
- **The analysis** is checked against jump tables and helper addresses worked out
  by hand elsewhere, and asserts that no block is ever claimed without an edge
  reaching it.
- **The instruction model** is checked against 4.2 million instructions another
  project emitted for these titles, comparing which registers each touches. The
  only disagreements left are two defects in that output: a recording vector
  comparison that never sets its condition field, and a vertex unpack it
  refuses outright.
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
make -C ./lifted -j8
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
- **Under a tenth of a percent of functions do not lift**, all of them blocked
  by a Direct3D vertex pack or unpack format that neither title's used forms
  cover. The two unpack formats these titles do reach are modelled; the rest
  have no reading here, and one built from a guessed format would produce
  plausible floats rather than an obvious mistake, so they stay refused.
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

## What is borrowed

Two things here could not be recovered from the artefact and are not original.

An Xbox 360 title imports from the kernel by ordinal and never by name. The
names are nowhere in the container: searching a decoded image for any of them
finds nothing, because nothing in the file ever held them. So the catalogue in
`crates/xenolith-xex/src/exports.rs`, which says that `xboxkrnl.exe` ordinal 204
is `NtAllocateVirtualMemory`, is transcribed from the export tables of the Xenia
project.

It is an interface catalogue rather than an implementation: what each entry
point is called and which number it answers to, the same kind of fact as a
system call number. No code of theirs is used, and nothing here says what any of
those entry points do. Their licence is reproduced in full at the head of that
file, which is where it has to be, and it forbids using their name to endorse
this, so nothing here should be read as their endorsement of it.

The second is the meaning of the Direct3D vertex unpack's type field. The field
is plainly there in the encoding and what each of its values selects is not, and
no amount of reading the image recovers it, because the image only ever uses the
formats rather than describing them. The two values these titles use were read
from the XenonRecomp project. That is a reading of an encoding rather than code
taken across, and because it was not derived here it was checked before it was
relied on: their vector storage is the guest's byte order reversed, and the
translation between their lanes and ours was pinned against the one unpack form
this project had already worked out independently, where the two agree in every
lane. The formats neither title uses are still refused.

Everything else in this repository is recovered from the container, checked
against the oracles above, or reported as unrecovered.

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
decoder. The kernel export catalogue and the vertex unpack's type field are the
two exceptions and are credited above.
