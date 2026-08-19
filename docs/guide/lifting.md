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
| `Makefile` | builds every unit and archives the objects |

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
make -C ./lifted -j
```

The larger of the two test titles builds in about two and a half minutes wall
clock on a machine with enough cores, from 88 units, at `-O2` with `-Wall
-Wextra`, with no warnings.

The makefile stops at `liblifted.a` rather than attempting a link. Nothing
implements the runtime interface, so a link would fail, and reporting that
failure as the build's outcome would say less than it seems to. An archive that
succeeds states exactly what has been achieved: the code compiles, and it is
waiting for a runtime.

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
functions               27447
  lifted                26908  (98.036 percent)
    import thunks         156
  not lifted              539
declarations            27569
units                      88
  largest             6015615 bytes
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
  mfmsr               233
  lvsl                 56
  vspltisw             49
  lvlx                 40
  dcbz                 24
```

This is the instruction that stopped the most functions, not the instruction
that appears most often, and the two are not the same list. Modelling the top
entry moves the coverage figure by roughly its count.

`--unlifted` lists every function that was not emitted, with the address and
mnemonic that stopped it, which is where to look once you want to know why.

## What it cannot do yet

The emitted program calls a runtime that does not exist. `xenolith.h` declares
the processor context, big endian memory accessors, and three entry points the
environment has to provide: a trap, an indirect dispatch for a branch whose
target was not known at lift time, and an import call.

Implementing those is the work that turns this into something that runs, and it
has not been started.
