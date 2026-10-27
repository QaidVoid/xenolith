# The runtime

The emitted C is written against `xenolith.h`, and the runtime is what
implements it. It stands in for the console: guest memory, the kernel and
system library calls a title makes, threads and synchronisation, the GPU,
sound, input and files. It is plain C, copied into every lift and compiled with
the emitted code.

| file | what it holds |
|---|---|
| `xenolith.h` | the interface the emitted code calls, see [the reference](/reference/interface) |
| `xenolith.c` | memory, threads, synchronisation, imports, files, and the files below |
| `command_stream.c` | reading the GPU command ring, included by `xenolith.c` |
| `drawing.c` | turning a draw command into a host draw, included by `xenolith.c` |
| `sound.c` | sound output and XMA decoding, included by `xenolith.c` |
| `vulkan.c` | the renderer and the window, built when Vulkan is present |

## Guest memory

The whole 4 GiB guest address space is one mapping, and a guest address is an
offset into it. A load is a read at `base + address`. The console also reaches
its physical memory through three mirrored ranges, so the lower 512 MiB is
mapped again at each of them, backed by the same shared memory: a word written
through one address is read through another.

The console is big endian. Loads and stores go through `xenolith_load32` and
its siblings, which assemble values from bytes in the guest's order, so the
emitted code never depends on the host's byte order.

There is no bounds check on a guest access, since one would cost on every load
and store. The sanitizer runs in the gate are what catch an access outside the
mapping.

## Allocation

Virtual and physical allocations come from one arena in 64 KiB grains. A size
that cannot be rounded into a grain is refused. Freed memory is not reused,
because nothing here knows what a title still holds.

## Threads

A guest thread is a host thread with its own register context and a 1 MiB
stack in guest memory. It reaches its processor block through `r13`, as on the
console: the block holds the processor number, the tick count and the state
the kernel keeps per processor. Thread local slots are numbered globally and
hold a value per thread.

## Synchronisation

Kernel objects a title creates (events, semaphores, timers, threads) are host
objects behind handles, and a wait is a condition variable wait. An object a
title keeps in its own memory and names by address is created on first use from
the header the title wrote.

Critical sections and spin locks are kept in the title's memory, in the
console's own layout, so a title that lays out or reads a section itself sees
what it expects. A free section counts its users as minus one, as on the
console, and a thread that has to wait sleeps on a host futex in the section's
header. A title that re-initialises a section while another thread waits on it
does not strand that thread: a waiter that finds the section started over takes
it.

## Imports

A title reaches the console's system libraries through thunks, which the lift
turns into calls to `xenolith_import` with the library and ordinal. The runtime
answers the ordinals it implements and reports the ones it does not. An import
is never silently answered with nothing.

Some answers depend on how a console was set up rather than on the title: the
video cable, display mode, region, language, signed in profile and the time
base rate. Each is answered with what the title's startup requires and reported
once when the run starts.

## The GPU

A title talks to the GPU by writing command packets into a ring buffer and
publishing how far it has written. A runtime thread reads the ring and carries
out the packets: register writes, shader constant loads, shader loads, draws,
copies and the memory writes a title waits on. Packets with no visible effect
are read for their length and counted.

### Shaders

A title hands over its shaders as the console's microcode. Every shader found in
the image is translated at lift time into SPIR-V in `shaders.c`, and looked up
by a hash of its words. A shader the title loads from its data files instead is
translated when it is first handed over, using the shader library copied into
the lift. See [Reading a shader](/internals/shader).

### Vertices

A console shader fetches its own vertices from memory by index. The runtime
copies the memory each draw's shaders read into a per frame buffer that the
translated shader reads, and passes where each stream begins beside the shader
constants. A table that many draws read, such as the per object transforms of a
city, is copied once a frame and shared by every draw that reads it.

Index lists are read as the title wrote them, including the console's 24 bit
indices and its restart value. Quads are drawn as pairs of triangles.
Rectangles, which the console describes with three corners, are finished on the
device by a geometry shader that adds the fourth corner after the vertex shader
has run.

### Render targets and copies

The console draws into a small on chip memory and copies (resolves) results out
to main memory, where they are used as textures or shown. The runtime keeps a
surface for each layout a title draws into, keyed by its pitch, sample count and
format, so that two passes sharing the same memory see each other's results. A
copy writes the surface back into guest memory in the console's tiled layout,
including depth in its 24 bit form, and refreshes any texture already made from
that memory.

### Textures

Textures are read from guest memory in the console's tiled layout and byte
order and uploaded in a format the host takes. Compressed formats are handed to
the device as BC1, BC2 and BC3 without decompressing.

### Frames

The renderer records one frame while the previous one runs on the device, with
two sets of per frame buffers. A frame the title asks to show is presented in a
window on X11 or Wayland. Without a window the run says so once and keeps
drawing off screen; `XENOLITH_FRAMES` writes shown frames to disk.

## Sound

A title mixes six channels and hands over a frame of 256 samples at a time,
paced by the runtime's answer to how far playback has got. The runtime folds
the frames to stereo and plays them through PulseAudio (including PipeWire's
PulseAudio server) or, failing that, ALSA. Both are loaded at run time.

Music and voices are XMA. A title fills in decoder contexts with two input
buffers and an output ring, and the runtime decodes them with the host's
libavcodec, loaded at run time. It follows the packets of the stream each
context reads, honours loop points, and moves the buffer offsets and valid
flags the way the console's decoder does. A title starts a context through
hardware registers the runtime cannot observe, so any context with input and a
ring to fill is decoded.

A host without a sound device or without libavcodec is reported once, and the
run continues silently.

## Input

The keyboard stands in for a controller: the arrow keys are the pad, `z` `x`
`c` `v` are A B X Y, enter is start, escape is back, `q` and `e` are the
shoulders, `1` and `3` pull the triggers all the way, `wasd` and `ijkl` push the
left and right sticks all the way, and `f` and `h` press them. Keys are read by
where they sit, so the letters are those of a QWERTY board. A run without a
window can be given held buttons with `XENOLITH_HOLD`.

## Files

A title's files are read from the directory the program runs in, through the
host's file calls. Saves go under `content/`.

## Reporting

Everything a run reports goes to stderr, one line per report, written under a
lock so lines from different threads do not interleave. The trace variables in
[Environment](/reference/environment) make a run report and carry on where it
would otherwise stop.
