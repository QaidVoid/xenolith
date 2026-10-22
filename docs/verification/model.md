# The model oracles

Three comparisons over what an instruction does, each at a different depth. They
are separate because they can see different things.

**What only they catch.** The corpus catches an instruction touching the wrong
register, over millions of instructions. The differential catches an instruction
computing the wrong value. The real function comparison catches control flow this
project would never have written, in code nobody here wrote.

## The emitted corpus

**Input.** 2,705,826 instructions another project emitted, from one title.

**Pass condition.** For each instruction, the set of registers this crate says it
reads and writes is the set the other project's output uses. 2,119,322
instructions are checked; the rest are instructions this crate does not model.

**Command.**

```sh
XENOLITH_LIFT_IMAGE=... XENOLITH_LIFT_CORPUS=... \
  cargo test --release -p xenolith-lift --test corpus -- --include-ignored
```

**What only it catches.** A field read from the wrong register, in families the
execution differential is far too slow to reach. It is shallow: it says which
registers are touched and nothing about the value.

**Limit.** The other project's output is a second implementation, so a mistake
both make is invisible. It also reaches no vector family the other project does
not emit, and it has itself been wrong once.

Where the two disagree, the run reports the count rather than failing, because a
disagreement between two implementations is a question to read rather than a
verdict. On the title tried there are 3. One of them is worth stating rather than
summarising: `vcmpgtuh.` is decoded with a record bit that writes condition field
6, and the other project's output does not write it. Either this crate is setting
a field the hardware does not, or the other project is dropping one. Nothing here
settles which, and it is recorded as open rather than counted as agreement.

## The execution differential

**Input.** One instruction encoding at a time, seeded with named registers and a
scratch page of patterned memory, assembled into a big endian ppc64 program and
run under an emulator. The same encoding is run through the C this project emits
for it, from the same state, on the host.

**Pass condition.** Every watched register, the condition register, the modelled
exception bits and a window of memory are identical afterwards.

**Command.**

```sh
XENOLITH_PPC_TOOLCHAIN=powerpc64-linux-gnu- \
  cargo test --release -p xenolith-lift --test execution -- --include-ignored
```

`scripts/check` runs this one itself when the toolchain and the emulator are both
installed, so having them and not using them is not a state the gate can be in.

**Decisions in it that were learned the hard way.** The instruction is placed as a
word rather than assembled from rendered text, which would test the renderer. Each
subject runs over several inputs, because one pair agrees by accident too often.
Scratch memory is seeded with a pattern rather than zeroes, so a load of the wrong
width brings back something visibly wrong. The seeds include values whose product
does not fit the register, so a fused form that rounds twice gives a different
answer from one that rounds once.

**Limit.** The emulator is a generic POWER, not a Xenon. Where the two differ the
harness reports the model wrong when it is right, which is why a disagreement is
read before it is believed. `mfmsr` and `mtmsrd` are privileged and will not run
in user mode, `mftb` returns a clock so there is nothing to compare, and `dcbz`
has an implementation defined block size.

## Real functions

**Input.** Leaf functions out of a shipped title, taken at an even stride across
the whole code section, run under the emulator against the real image and through
the emitted C on the host.

**Pass condition.** The volatile registers, the condition register, the modelled
exception bits and a window of memory agree.

**Command.**

```sh
XENOLITH_TITLE_SAMPLE=2000 ... \
  cargo test --release -p xenolith-lift --test title -- --include-ignored
```

**What only it catches.** Control flow. A branch's effect is on which instruction
runs next, which no single instruction's registers can show, and the code for one
is written by the emitter from the graph rather than by the model from the
encoding. This is where a conditional return emitted as an unconditional one, a
counted loop that never decrements, and a trap made unconditional were all found.

**Limit.** The console runs its titles in thirty two bit mode, where an effective
address is truncated before use, and `qemu-ppc64` truncates nothing. An address a
title formed by sign extending something at or above two gigabytes arrives with
its top half set, and the function faults on the hardware side. About three
quarters of a sample is lost this way, which is why the sample is large rather
than why the result is weak: what does run is compared exactly.
