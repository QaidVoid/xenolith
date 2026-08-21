---
layout: home
title: xenolith
---

## What it does today

It turns a retail game into C that compiles. It does not yet turn one into a
game you can play, and the gap between those two things is worth being precise
about.

It links, and running it gets as far as the first call into the operating
system. A runtime ships with it that maps guest memory, loads the image, and
implements what the interface declares. What it does not do is service an
import, create a thread, or draw anything.

So the translation is complete enough to link twelve thousand functions and run
them, and there is no environment underneath for them to run in.

Two registers are modelled as storage whose architectural effects are not
honored, which is a deliberate exception to the rule that nothing is
approximated. [Emitting C](/internals/lifting) says which and why.

Measured against two retail titles:

| | title A | title B |
|---|---|---|
| functions discovered | 32,738 | 14,558 |
| executable words claimed | 97.7% | 92.8% |
| functions lifted to C | 32,727 (99.97%) | 14,548 (99.93%) |
| of those, import thunks | 156 | 188 |
| emitted C compiles | yes, all of it | yes, all of it |

What is left is 21 functions across both titles, every one stopped by a Direct3D
vertex pack or unpack format neither title's used forms cover. The two unpack
formats the titles do reach are modelled; the rest have no reading here.

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
