# What this is

xenolith is a static recompiler for Xbox 360 executables. It reads a XEX
container, decodes the 64-bit PowerPC inside it, works out where the functions
are, and writes C.

It is Linux only, and deliberately so. There is no Windows support and none
planned.

## Static, not emulated

An emulator decodes an instruction every time it runs one. A static recompiler
decodes each instruction once, ahead of time, and writes out code that does the
same thing. The result is a program rather than an interpreter, so the cost of
decoding is paid at build time and never again.

The price is that everything has to be worked out without running anything. An
emulator always knows where it is; a recompiler has to find every function, every
basic block, and every branch target by reading the bytes. Where a branch goes
through a register, it has to reconstruct the table the program would have read.

That is most of what this project is.

## No per title configuration

The comparable tool for this job requires a hand written file per game. That
file names every function boundary the tool could not find, the addresses of the
eight register save and restore helpers, the addresses of `setjmp` and
`longjmp`, and byte patterns to skip. A companion tool produces a second file
listing every jump table, which for one title runs to 852 entries across 30,250
lines.

xenolith accepts none of that. There is no configuration file, no title
database, and no place to write an address by hand. Everything is recovered from
the image, or reported as unrecovered.

This is a constraint rather than a convenience. A tool that can be told the
answer will be told the answer, and then nobody finds out that it could not work
it out. Removing the option is what makes the coverage figure mean something.

## What it will not do

- **It will not run your game.** The runtime interface is a declaration. Nothing
  maps guest memory, services an import, or creates a thread.
- **It will not guess.** An instruction the decoder does not recognize is
  reported as unknown. A function holding an instruction the semantic model
  cannot express is not emitted at all, because code that is right except in one
  place compiles and runs and is wrong, and nothing downstream can tell.
- **It will not decrypt a title for you.** No key material ships with it. See
  [Key material](/guide/keys).

## The shape of the pipeline

Four crates, each usable on its own:

| crate | what it does |
|---|---|
| `xenolith-xex` | reads the container: headers, page descriptors, imports, decryption, decompression |
| `xenolith-ppc` | decodes 440 instructions across 64-bit PowerPC, AltiVec, and VMX128 |
| `xenolith-analysis` | finds functions, blocks, edges, register helpers, and jump tables |
| `xenolith-lift` | describes what each instruction does and emits C for a whole function |

The `xenolith` binary exposes them as four subcommands. Each stage is checked
against something produced independently before the next one is built on it,
which is the subject of [How it is checked](/verification).
