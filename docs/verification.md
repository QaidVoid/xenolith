# How it is checked

Nothing here is trusted because it looks right. Every layer is compared against
something produced independently, and each oracle is kept because there is
something only it can see.

## The oracles

| layer | checked against | what only it catches |
|---|---|---|
| container | two retail titles, plus a reference image from another implementation | decryption and reconstruction together, against ground truth |
| decoder, mnemonics | `llvm-mc` | an instruction named as the wrong one |
| decoder, operands | GNU `objdump` | a field read from the wrong bits |
| analysis | jump tables and helper addresses worked out by hand elsewhere | a function or target that should have been found |
| instruction model | 1.19 million instructions another project emitted | an instruction touching the wrong register |
| semantics | PowerPC executed under emulation | an instruction computing the wrong value |

## What each one found

**GNU objdump found eight rendering bugs.** Naming an instruction correctly says
nothing about whether its fields were read correctly, and the mnemonic
comparison could not see any of them. Condition register operands, unsigned
logical immediates, branch condition fields, and float operands were all being
printed from the wrong bits.

This comparison has a blind spot worth stating: a field printed as the wrong
kind of thing, where the number coincides, is invisible to a comparison of
values. `srawi r4, r3, r4` printed a shift amount as a register and was found by
reading, not by the oracle.

**The emitted corpus found nine model bugs.** It compares which registers each
instruction reads and writes, over 1.19 million of them, which reaches families
the execution differential is far too slow to touch.

**Execution found six real semantic mistakes on its first run.** The worst was
that the record bit compared the low 32 bits of a result rather than all 64, so
every dot suffixed instruction set the wrong condition whenever a result's
halves disagreed in sign. Nothing else built could have seen it: the corpus says
which registers are touched, objdump says which fields exist, and neither can
say that a comparison used the wrong width.

**It later found a seventh by hanging.** A conditional store's low encoding bit
is part of its spelling rather than a record bit, and the emitter read it as one.
The generic path appended a comparison of the stored value against zero, which
overwrote the condition field the store had just set from its own outcome. The
retry branch after it then read a bit that always said the store had failed, and
the emitted code looped forever.

Reading that code would have shown two writes to the same field and looked like
harmless duplication. The corpus would have agreed with it, since the touched set
was right either way. Only running it showed the loop, and it showed it by not
finishing, which is a symptom worth taking as seriously as a disagreement.

## The execution differential

This is the strongest oracle and the newest, so it is worth describing.

An instruction encoding is placed as a word in an assembly file the harness
writes, with every register seeded by name and every register that could have
changed stored afterwards, along with the condition and exception registers and
a window of scratch memory. That is built with a big endian ppc64 cross
toolchain and run under a user mode emulator. The same encoding is run through
the C this project emits for it, from the same state, on the host. The two are
compared.

Several decisions in it were learned the hard way.

**Registers are named in assembly the harness writes.** The first prototype used
inline assembly with register constraints, and the compiler allocated its own
temporaries over the seeded registers. It reported that adding two numbers
returned one of them.

**The instruction is placed as a word, not assembled from text.** Assembling
rendered text would test the renderer rather than the decoder, and would fail for
the families this project deliberately prints in encoding order.

**Several inputs per instruction.** A single input pair agrees by accident too
often. Zero and zero agree for nearly every operation, and two small positive
numbers hide every sign and overflow question. The seeds include negatives,
values whose low word disagrees with their high, and values that carry out.

**Scratch memory is seeded with a pattern rather than zeroes.** A load of the
wrong width or byte order brings back something visibly wrong from a pattern and
something indistinguishable from a buffer of zeroes.

**Control flow is checked by running whole functions.** A branch's effect is on
which instruction runs next, which no single instruction's registers can show,
and the code for one is written by the emitter from the graph rather than by the
model from the encoding. Short functions are run on hardware and lifted and run
on the host, and the harness checks that the outcomes actually differ across
inputs, because two sides can agree while neither took a path.

**Only the modelled bits of the exception register are compared.** The emulator
reports bits this project does not model and lifted code never reads.

## What this cannot reach

**The console's vector extension has no execution oracle.** No assembler accepts
VMX128 and no emulator implements it. Those instructions have the emitted corpus
and careful reading behind them, and nothing more. A coverage figure that folded
them in would imply more than it means.

**The emulator is a generic POWER, not a Xenon.** Where the two differ, the
harness would report the model wrong when it is right. That is a real limit and
the reason a disagreement is read before it is believed. Two of the first three
disagreements found were the harness and the test data rather than the model.

Some instructions are excluded for that reason. `mfmsr` and `mtmsrd` are
privileged and a user mode emulator will not run them. `mftb` returns a clock,
so there is nothing to compare. `dcbz` operates on a block size that is
implementation defined, and the emulator's is not this console's, so its size is
taken from the documentation and checked against nothing.

**Coverage is a third of the instruction set, deeply.** The differential
executes a few hundred encodings on several inputs each. The corpus covers a
million instructions shallowly. Neither replaces the other.

## Property tests and fuzzing

Property tests assert things that have to hold for any input: truncating a
container at every length yields an error and never a panic; a read of the
address space either returns the length asked for or fails.

Fuzz targets cover the loader, the decoder, the analysis, and the lifter. A fuzz
target on the analysis checks the reachability of resolved flow, so a recovered
jump table cannot introduce a target nothing branches to.

## Running the real oracles

The suite passes on a clean checkout with no game data present, which is a
weaker statement than it looks. Tests needing a real title, a cross toolchain, or
an emulator read paths from the environment and skip when they are unset.

[Environment](/reference/environment) lists every variable and what it names.
Run those with `--release`; in a debug build they take minutes.
