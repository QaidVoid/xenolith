# The runtime

The emitted C is written against `xenolith.h`, which this project ships. That
interface is what makes a title's code run: it maps guest memory, loads the
image, answers the kernel entry points a title asks for, and reads the commands a
title writes for the graphics hardware.

The runtime is ordinary C, included into one translation unit by `xenolith.c`,
and it is compiled with the emitted code rather than linked as a library.

## Guest memory

The whole four gigabyte guest address space is mapped once, read and write,
anonymously. A guest address is an index into that mapping, so a load is a byte
read at `base + address` and nothing else.

That is what makes the runtime dangerous to itself: there is no bounds check
anywhere, because a check would cost the one thing this is measured on. The
sanitizer run in the gate is what catches a read or write outside the mapping,
and it is the reason that run exists.

Loads and stores go through `xenolith_load8` and its siblings, which assemble a
value from bytes in the guest's own byte order. A `stw` is four byte stores, not
a host store, because the host may not share the guest's order and the guest's
memory is a byte array rather than a typed one.

## Allocation

Two allocators, one arena. A guest address handed out for virtual memory and one
handed out for physical memory come from the same run of grains, and a grain is
64 KiB.

A size that cannot be rounded into a grain is refused rather than handed back as
the nought it rounds to, which a title would take for an answer.

Nothing is ever given back. Freeing an address is answered with success and the
memory is not reused, which is written down where the memory is handed out. A
runtime that reused it would have to know what the title still holds, and nothing
here does.

## Threads

A guest thread is a real host thread with its own register state and stack. Each
gets a megabyte of the guest address space for its stack, taken from the top
downward.

A thread reaches its own state through `r13`, which holds a block of its own:
the processor number, a counter, and the shared word a title spins on. The first
thread a title runs on needs the same block, and the boot program sets it, which
is a thing that was missing and showed up as a title reading processor nought on
every thread.

Waits are condition variables. A wait with a timeout is a timed wait, and a
pulse releases every waiter.

Thread-local slots are numbered across all threads. A title puts something in a
slot on one thread and expects to read it back on that thread and nowhere else,
so one set shared between threads gives every thread the last value written by
any of them.

## Imports

A title reaches the operating system by calling a thunk. The thunk's first two
words are placeholders the loader overwrites, so the lift emits a call to the
runtime's import entry point instead, naming the library and the ordinal.

The runtime answers the ordinals it implements and reports the ones it does not.
An import is not guessed at: answering one with nothing when it was meant to
allocate memory or open a file is a lie the title will believe, and everything
after it describes the answer it was given rather than the title.

What each answer rests on is stated when the run starts, if it rests on anything
but the container. A privilege, a display setting, the video cable and the
timebase rate are all assumptions, and each is reported once, in a line naming
what was assumed and why it could not be recovered.

## The graphics command stream

A title does not call the graphics hardware. It writes packets into a ring buffer
and publishes where it has got to by storing into a mapped register. A thread in
the runtime reads the ring, walks the packets, and carries out the ones a title
can see the effect of.

A packet's length field is one less than the number of words after the header,
which is checkable against a real buffer: the lengths land exactly on the next
header, four packets running, with nothing left over.

What a packet does depends on the opcode. Setting registers, loading constants
into the shader constant block, setting up a buffer or a shader, and drawing are
read. Anything else is read for its length and passed over, and a run says how
many it passed over rather than pretending it drew them.

A packet whose effect a title can see, which is a write to an address it named,
is carried out, because a title waits on that and never carries on without it.

## Drawing

The renderer opens the host's device, translates a title's own shaders, and
issues the draws the command stream asks for. It is in `vulkan.c`, and it is
built only when the build has Vulkan.

A draw is read out of the packet and turned into what the renderer is asked for:
the vertex buffer, the stride, the shader pair, the viewport, the scissor, and
the shape. The shaders are a title's own, translated from the words it handed
over, and looked up by the sum of those words.

Where the host has no way to show a frame, the run says so in one line and
carries on drawing offscreen. That is a limit of the host and not a fault in the
title.

## Sound

A title hands over a mixed frame and expects to be told it was taken, because
that answer is the clock it paces itself by. The runtime folds the console's six
channels to two and plays them through the host's ALSA, loaded at run time.

A host with no device is reported once and the run carries on without sound.

## Input

A controller is read from the desktop's own way of reading a keyboard, because a
host without a console controller still has to give a title something. The arrow
keys are the pad, `z` and `x` are the two face buttons, and enter is start.

A run with no window can be told what is held with `XENOLITH_HOLD`, which is what
takes a title past the screen that waits for a button.

## Files

A title's own files are read from beside the program, through the host's own
file calls. A file is opened, read, written, and closed with the semantics the
title asked for, and the mapping between a guest handle and a host one is a
table in the runtime.

## Reporting

A run says what it did rather than only what it did not. The graphics and sound
reports, the assumptions, and the end-of-run count of what was read are all on
stderr, one line each, under a lock so a line arrives whole from whichever thread
wrote it.

The trace variables in [Environment](/reference/environment) make a run report
and carry on instead of stopping, for the cases where a list of what a title
wanted is more useful than a stop.
