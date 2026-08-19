---
layout: home
title: xenolith
---

## What it does today

It turns a retail game into C that compiles. It does not yet turn one into a
game you can play, and the gap between those two things is worth being precise
about.

The emitted C is written against a runtime interface this project declares and
does not implement. No guest memory is mapped, no import does anything, no
threads exist. Around 1,460 addresses are declared without a definition, mostly
functions that could not be lifted plus the register save and restore helpers.
So it compiles, and it does not link.

Measured against two retail titles:

| | title A | title B |
|---|---|---|
| functions discovered | 27,447 | 11,903 |
| executable words claimed | 95.3% | 86.9% |
| functions lifted to C | 26,908 (98.0%) | 11,195 (94.1%) |
| of those, import thunks | 156 | 187 |
| emitted C compiles | yes, all of it | yes, all of it |

## The point of it

An existing tool does this job and requires, per game, a hand written file
naming every function boundary, the addresses of eight register helpers, setjmp
and longjmp, and byte patterns to skip. Its companion emits a separate file of
jump tables, which for one title runs to 852 entries across 30,250 lines.

This project accepts no per title configuration at all. Everything above is
recovered, or reported as unrecovered. Nothing is guessed.

- All eight register helper addresses are detected on both titles without being
  given any of them, matched against values two other projects recorded by hand.
- 803 of the 852 jump tables are recovered and agree exactly with what that tool
  produced, with zero disagreements.
- Every import record is read from the image and reported as a library and an
  ordinal.
- An instruction the decoder does not recognize reports itself unknown, and a
  function holding an instruction the model cannot express is not emitted at all.

## Nothing here is trusted because it looks right

Every layer is compared against something produced independently: the decoder
against `llvm-mc` and GNU `objdump`, the analysis against jump tables worked out
by hand elsewhere, the instruction model against 1.19 million instructions
another project emitted, and the semantics against PowerPC hardware running the
same instruction the emitted C does.

That last one found six real mistakes on its first run, including one where
recording a result compared the low half of a register rather than the whole of
it. [How it is checked](/verification) goes through each oracle and what only it
can see.

## Where to go next

- [Getting started](/guide/getting-started) builds it and runs it against a file.
- [Key material](/guide/keys) explains why a retail title needs a key and what
  that key actually is.
- [How it works](/internals/container) walks the pipeline from container to C.
- [Commands](/reference/cli) is the full surface of the tool.
