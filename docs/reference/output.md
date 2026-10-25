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
| `drawing.c` | the part that turns a draw command into what the renderer is asked for |
| `sound.c` | the part that plays the sound a title mixes, and decodes the compressed sound it mixes from |
| `vulkan.c` | the renderer, which turns those commands into host draw calls |
| `shaders.c` | every shader the image holds, translated, where the runtime finds them by the words a title hands over |
| `unlifted.c` | a trap for every address that could not be lifted |
| `table.c` | every lifted function, for an indirect branch to resolve against |
| `main.c` | boots the runtime and enters the guest |
| `image.bin` | the decoded image, which the runtime loads |
| `Makefile` | builds every unit and links them into a program |

Three of those come and go with the machine and the title.

| file | when it is there |
|---|---|
| `libxenolith_shader.a` | when the build that made `xenolith` left one beside it. A title loads some shaders from the data beside its executable rather than out of the executable, and reading the executable cannot find one of those |
| `xdg-shell-client-protocol.h` | when the machine has `wayland-protocols` and `wayland-scanner`. It is not emitted: `make` generates it from the description the desktop publishes |
| `xdg-shell-protocol.c` | the same, and compiled into the program |

`command_stream.c`, `drawing.c` and `sound.c` are included by `xenolith.c`
rather than compiled on their own, so they are not listed in the makefile's
sources. They go beside the runtime because that is where it looks for them.

Where the shader library is not there, the lift says so once and names the path
it looked at.

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
| `CFLAGS` | `-O2 -Wall -Wextra -Wno-infinite-recursion` by default, plus a switch for each capability found |
| `LDLIBS` | `-lm -lpthread` by default, plus a library for each capability found |
| `SOURCES` | the units, `unlifted.c`, `table.c`, `shaders.c`, `xenolith.c`, `vulkan.c`, the generated protocol file and `main.c` |

Three capabilities are looked for in one block, and each contributes a switch.

| looked for | asked of | switch | library |
|---|---|---|---|
| `vulkan` | `pkg-config` | `-DXENOLITH_WITH_VULKAN` | `-lvulkan` |
| `x11` | `pkg-config` | `-DXENOLITH_WITH_X11`, and `-DXENOLITH_WITH_WINDOW` | `-lX11` |
| `wayland-client`, `wayland-protocols`, `wayland-scanner` | `pkg-config` and the path | `-DXENOLITH_WITH_WAYLAND`, and `-DXENOLITH_WITH_WINDOW` | what `pkg-config` says |

`XENOLITH_WITH_WINDOW` is set where either way of asking a desktop for a window
is there, because a title that only ever draws does not care which. Turning the
three off by hand gives a build with no graphics at all:

```sh
make -C ./lifted DRAWING=no SHOWING=no TALKING=no
```

`SAYING` is the one capability decided apart from those, because it is about a
file beside the emitted code rather than about the machine: `$(wildcard
libxenolith_shader.a)`.

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