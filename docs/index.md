---
layout: home
title: xenolith
---

## What it does

xenolith turns an Xbox 360 game into a native Linux program. It reads the
game's XEX executable, finds every function in it, translates the PowerPC code
to C, and builds that C together with a runtime that stands in for the
console: memory, the kernel calls a game makes, threads, the GPU, sound, input
and files.

It needs no per game configuration. Everything a comparable tool asks you to
write down by hand is recovered from the executable instead:

- **Function boundaries**, from calls, prologues, the image's own unwind
  records and tables of function pointers.
- **Jump tables**, by abstract interpretation of the code leading up to each
  indirect branch. 803 of one title's 852 tables are recovered and agree exactly
  with a hand built reference.
- **Register save and restore helpers**, by their shape. All eight are found on
  both test titles.
- **Imports**, read from the container as a library and an ordinal.

What cannot be recovered is reported, never guessed.

## Where it stands

Two retail titles lift completely, compile without warnings, boot, and draw
with their own translated shaders. The first is played into its first mission
with music and voices. Some scenes still render wrong and it is not playable end
to end. The [README](https://github.com/QaidVoid/xenolith#status) has the
current counts.

## Checked against independent sources

Every stage is compared against something it did not produce: the decoder
against `llvm-mc` and GNU `objdump`, discovery against hand worked jump tables,
the instruction model against millions of instructions another project emitted,
and the emitted C against real PowerPC code running under emulation.
[How it is checked](/verification/) covers each comparison and what only it can
catch.

## Where to go next

- [What this is](/guide/) explains the approach and the pieces.
- [Getting started](/guide/getting-started) builds it and runs a title.
- [How it works](/internals/container) walks the pipeline stage by stage.
- [Commands](/reference/cli) and [Environment](/reference/environment) are the
  reference.
