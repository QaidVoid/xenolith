# Lifting a title

`lift` is the subcommand that produces C. This page covers what it writes, what
that output can and cannot do, and how to read the report.

## What lands in the output directory

```sh
xenolith lift default.xex --out ./lifted
```

| file | what it is |
|---|---|
| `lifted.<first>-<last>.c` | a translation unit, named after the address range of the functions in it |
| `lifted.h` | every function declaration, written once |
| `xenolith.h` | the runtime interface the emitted code is written against |
| `xenolith.c` | a runtime implementing that interface and nothing more |
| `unlifted.c` | a trap for every address that could not be lifted |
| `table.c` | every lifted function, for an indirect branch to resolve against |
| `main.c` | boots the runtime and enters the guest |
| `image.bin` | the decoded image, which the runtime loads |
| `Makefile` | builds every unit and links them into a program |

A retail title emits hundreds of megabytes of C. As one file that is a
translation unit no compiler can build in reasonable time, and a C build
parallelizes across translation units and nowhere else, so one file also means
one core. The output is split for that reason, at a byte budget rather than a
function count, since function sizes span several orders of magnitude and a
fixed count per unit would produce units whose compile times differ as widely.

A function is never split across units. The budget is checked between functions,
so a unit ends slightly over it rather than part way through a body.

```sh
xenolith lift default.xex --out ./lifted --part-size 0x200000
```

## Building it

```sh
make -C ./lifted -j8
```

The larger of the two test titles builds in about two and a half minutes wall
clock on a machine with enough cores, from 91 units, at `-O2` with `-Wall
-Wextra`, with no warnings.

The makefile links into a program called `lifted`. That matters for more than
tidiness: compiling a unit says it is well formed on its own, and only a link
says twelve thousand functions agree with each other about what exists and that
nothing is defined twice. Nothing else checks that.

```sh
./lifted image.bin 0x82090000
```

The program takes an image and an address, so you can enter anywhere rather than
only at the recorded entry point. Entering a title at its entry point reaches
something unimplemented and stops, naming the address.

## What the emitted C looks like

Each function becomes a C function named from its address. Each basic block
becomes a label and each edge a branch to one. Every instruction is emitted
under its own disassembly, so the two can be read against each other.

```c
/* 0x82090000 */
void sub_82090000(xenolith_context *ctx, uint8_t *base) {
    uint32_t address;
    goto loc_82090000;

loc_82090000:;
    /* 0x82090000  mfspr r12, r8, r0 */
    ctx->r[12] = ctx->lr;
    /* 0x82090004  stw r12, -8(r1) */
    address = (uint32_t)ctx->r[1] + (uint32_t)(-8);
    xenolith_store32(base, address, (uint32_t)ctx->r[12]);
    /* 0x82090020  cmpli cr6, 0, r10, 0 */
    ctx->cr[6].lt = 0;
    ctx->cr[6].gt = ((uint64_t)(uint32_t)ctx->r[10]) > (0ull);
    ctx->cr[6].eq = ((uint64_t)(uint32_t)ctx->r[10]) == (0ull);
    ctx->cr[6].so = (uint8_t)(ctx->xer >> 31) & 1;
    /* 0x82090028  bc 12, 26 0x82090034 */
    if (ctx->cr[6].eq) { goto loc_82090034; }
    goto loc_8209002c;
```

Nothing is reordered and nothing is optimized. Blocks become labels because that
is what the control flow graph says, and anything else would be a transformation
that has to be justified. The C compiler is better placed to optimize this than
the emitter is.

## Reading the report

```
functions               32738
  lifted                32727  (99.966 percent)
    import thunks         156
  not lifted               11
helper entries             50
declarations            32868
units                     106
  largest             4809077 bytes
```

**functions** is what discovery found. **lifted** is how many were emitted.
**import thunks** counts those that are calls into the operating system rather
than translated code, reported apart because an import was never going to be
lifted and counting it either as a success or as a failure overstates something.

**declarations** exceeds the function count because emitted code names more than
discovered functions: a call into a register save helper lands part way through
one, and discovery does not claim those. Every name is declared or the C would
not compile.

## Finding what to work on

```sh
xenolith lift default.xex --out ./lifted --blockers
```

```
instructions blocking the most functions
  vpkd3d128             6
  vupkd3d128            5
```

This is the instruction that stopped the most functions, not the instruction
that appears most often, and the two are not the same list. Modelling the top
entry moves the coverage figure by roughly its count.

`--unlifted` lists every function that was not emitted, with the address and
mnemonic that stopped it, which is where to look once you want to know why.

## Feeding a run back

A built program that reaches an address nothing was lifted for says where it was
going. Setting `XENOLITH_TRACE_DISPATCH` makes it report every such address and
carry on, rather than stopping at the first:

```sh
XENOLITH_TRACE_DISPATCH=1 ./lifted 2>&1 | grep '^dispatch ' | awk '{print $2}' | sort -u > roots.txt
xenolith lift default.xex --out ./lifted --roots-from roots.txt
```

This is worth doing when the trace names an address inside the image. Such an
address is a function that discovery did not find, and feeding it back is the
evidence discovery could not have had. A function reached only through a table
the title writes while it runs leaves nothing to find it by: no prologue, and no
branch naming it.

It is worth doing only then. A trace also names addresses that are not code at
all, and those are the more common case once a title is failing: a value the
title wrote into its own memory and then read back as a function pointer, or an
address outside the image entirely. Handing one of those to `--roots-from` adds
nothing, because there is no code there to lift.

```sh
xenolith disasm default.xex --start 0x82871af0 --length 4
# error: 0x82871af0 is not in an executable section
```

An address the image does not hold is a title in a failure state rather than a
gap in the translation. The question it raises is which step of the title's own
initialisation did not happen, and no amount of lifting answers it.

## What it cannot do yet

The runtime that ships with it maps guest memory, loads the image, resolves an
indirect branch against the table, services the kernel entry points a title's
startup reaches, and reports anything it cannot do. Guest threads are real host
threads with their own register state and stack.

Both titles run their whole startup, every static constructor, and every kernel
entry point they ask for. What they do not do is play. On the title this has been
taken furthest with, the renderer opens the host's device, translates the title's
own shaders, reads the command stream it writes, and issues its draws, and the
picture that comes out is black. The title stops by dispatching to an address
outside its own image, which is its own failure state rather than a gap in the
translation. The other title tried gets less far, and the runtime reports which
entry point it asked for that nothing answers. Both are recorded in the project's
research notes rather than here, because each is a finding about one title rather
than a property of the translator.
