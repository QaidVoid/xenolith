# What a lift writes

`xenolith lift` turns a title's executable into C and the runtime that C is
built against. What lands in the output directory is listed here, and a test
checks that this list and the directory agree.

## The files

| file | what it is |
|---|---|
| `lifted.<first>-<last>.c` | a translation unit, holding the functions whose addresses fall between the two named |
| `lifted.h` | every function declaration, written once |
| `xenolith.h` | the interface the emitted code is written against |
| `xenolith.c` | the runtime, implementing that interface |
| `command_stream.c` | the part of the runtime that reads a title's commands for the graphics hardware |
| `sound.c` | the part of the runtime that plays the sound a title mixes |
| `unlifted.c` | a trap for every address that could not be lifted |
| `table.c` | every lifted function, for an indirect branch to resolve against |
| `main.c` | boots the runtime and enters the guest |
| `image.bin` | the decoded image, which the runtime loads |
| `Makefile` | builds every unit and links them into a program |

`command_stream.c` and `sound.c` are included by `xenolith.c` rather than
compiled on their own, so they are not listed in the makefile's sources. They go
beside the runtime because that is where it looks for them.

## The units

A retail title emits hundreds of megabytes of C. As one file that is a
translation unit no compiler can build in reasonable time, and a C build
parallelises across translation units and nowhere else, so one file also means
one core.

The output is split at a byte budget rather than a function count, because
function sizes span several orders of magnitude and a fixed count per unit would
produce units whose compile times differ as widely. A function is never split
across units: the budget is checked between functions, so a unit ends slightly
over it rather than part way through a body.

The name of a unit is the range of addresses it holds, in hexadecimal, so
`lifted.82090000-820a54d4.c` holds the functions from `0x82090000` up to but not
including `0x820a54d4`.

## The makefile

Four variables, all of which can be overridden on the command line.

| variable | what it holds |
|---|---|
| `CC` | the compiler, `cc` by default |
| `CFLAGS` | `-O2 -Wall -Wextra -Wno-infinite-recursion` by default |
| `LDLIBS` | `-lm -lpthread` by default |
| `SOURCES` | the units, `unlifted.c`, `table.c`, `xenolith.c` and `main.c` |

`SOURCES` and `OBJECTS` are worked out from each other, so adding a file means
adding it to `SOURCES` and nothing else.

`make -j8` builds in parallel and links. Give `-j` a number: a job for each unit
is more than a machine has.

## What the program does

It links and it does not play a game. The first import the guest reaches ends
it, which is an accurate account of how far the translation has got.

Building it matters for more than tidiness. Compiling a unit says it is well
formed on its own; only a link says the units agree with each other about what
exists and that nothing is defined twice. Nothing else checks that.

It takes the image and an address, so you can enter anywhere rather than only at
the recorded entry point:

```sh
./lifted image.bin 0x82090000
```
