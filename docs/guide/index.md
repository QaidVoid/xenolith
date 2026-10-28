# What this is

xenolith is a static recompiler for Xbox 360 executables. It reads a XEX
container, decodes the 64-bit PowerPC code inside it, finds the functions,
writes C for them, and ships a runtime that lets that C run as a native Linux
program.

It is Linux only.

## Static, not emulated

An emulator decodes each instruction every time it runs it. A static
recompiler decodes each instruction once, ahead of time, and writes code that
does the same thing. The result is an ordinary program that a C compiler
optimises, with no decoding left at run time.

The cost is that everything has to be worked out without running anything. An
emulator always knows where it is. A recompiler has to find every function,
block and branch target by reading the bytes, and where a branch goes through a
register it has to reconstruct the table the program would have read. That is
most of what this project does.

## No per title configuration

The comparable tool for this job needs a hand written file per game naming the
function boundaries it could not find, the addresses of the register save and
restore helpers, `setjmp` and `longjmp`, and byte patterns to skip, plus a
second generated file of every jump table.

xenolith has no configuration file, no title database and no place to write an
address by hand. Everything is recovered from the image or reported as
unrecovered. Removing the option is deliberate: a tool that can be told the
answer never shows whether it could have found it.

## Refuse rather than guess

- An instruction the decoder does not recognise is reported as unknown.
- A function containing an instruction the model cannot express is not emitted
  at all. A function with one wrong instruction compiles, runs and misbehaves
  somewhere nothing points to; a refused function is a number you can act on.
- A shader using something the translator cannot express is refused, and the
  draws that use it are skipped and reported.
- Where the runtime has to assume something about the console, such as the
  video cable or the region, it says so when the run starts.

## The pieces

| crate | what it does |
|---|---|
| `xenolith-xex` | reads the container: headers, page descriptors, imports, decryption, decompression |
| `xenolith-ppc` | decodes 64-bit PowerPC, AltiVec and the console's VMX128 extension |
| `xenolith-analysis` | finds functions, blocks, edges, register helpers and jump tables |
| `xenolith-lift` | models what each instruction does, emits C, and holds the runtime |
| `xenolith-shader` | translates the console's GPU shaders to SPIR-V |
| `xenolith-host` | the one library a built game links: it reads the game from the player's copy and translates shaders the game loads while running |
| `xenolith` | the command line tool tying them together |

The runtime lives in `crates/xenolith-lift/runtime/` and is copied into every
lift, so the output directory builds on its own.

## What it will not do

- **Decrypt a title for you.** No key material ships with it. See
  [Key material](/guide/keys).
- **Model what no title uses.** Coverage follows what the two test titles
  exercise. Anything else is refused and reported.
- **Run on Windows or macOS.**
