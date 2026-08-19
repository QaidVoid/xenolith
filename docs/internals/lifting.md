# Emitting C

`xenolith-lift` is where a discovered function becomes something that compiles.
It is built in two layers that are deliberately kept apart.

## Effects and code

An instruction's **effect** says what it touches: which registers and condition
fields it reads, which it writes, and in which bank. An instruction's **code** is
the C that does it.

They are separate because they can be checked in completely different ways. What
an instruction touches can be compared against an independent corpus, mechanic-
ally, over a million instructions. What it computes cannot, because deciding
whether two expressions are equivalent is harder than the problem being solved.

So an instruction may have an effect and no code. A function is emitted only when
every instruction in it has both.

## Whole or not at all

A function containing an instruction the model cannot express is not emitted. It
is reported, naming the function, the address, and the mnemonic that stopped it.

Emitting a function with a hole is worse than emitting nothing. It compiles, it
runs, and it is wrong, and nothing downstream can tell the difference. A refusal
is a number you can act on.

## The shape of the output

One C function per discovered function, named from its address so a caller can
be matched to a callee without a table on the side. Each basic block is a label
and each edge a branch to one. Where a block falls through, the fall through is
written out explicitly, so reordering the blocks cannot change what the code
does.

Every instruction is emitted under its own disassembly. That doubles the volume
of the output and it is the only review this code can get until it can be run.

Control leaving the function:

| in the guest | in the C |
|---|---|
| call | link register set, then a call to the target's function |
| return | `return` |
| tail call | a call, then a return, so the callee does not reuse the caller's frame |
| indirect branch, table recovered | a branch among those targets |
| indirect branch, no table | a call through the runtime's indirect dispatch |
| import thunk | a call to the runtime's import entry point |

## The runtime interface

The emitted code is written against `xenolith.h`, which this project ships and
does not implement.

It declares a processor context holding the general purpose registers, the
floating point registers, the vector registers, the condition register fields,
and the link, count, and exception registers. Floating point storage is a union,
because the instruction set writes a value at one width and reads it at another
and an emitter that could not express that would have to guess which was meant.

Memory is reached through accessors that assemble bytes explicitly, so the
emitted code holds the guest's byte order rather than the host's.

These entry points are declared for the environment to provide:

```c
void xenolith_trap(xenolith_context *ctx, uint8_t *base, uint32_t address);
void xenolith_dispatch(xenolith_context *ctx, uint8_t *base, uint32_t address);
void xenolith_import(xenolith_context *ctx, uint8_t *base, const char *library,
                     uint32_t ordinal);
uint32_t xenolith_reserve32(const uint8_t *base, uint32_t address);
uint8_t xenolith_conditional32(uint8_t *base, uint32_t address, uint32_t value);
uint64_t xenolith_timebase(void);
```

A trap leaves the function it was in, so what happens next is not something
emitted code can express. A dispatch is the single place an address unknown at
lift time becomes a function. An import is a call into what the console
provided. A reservation goes through the runtime rather than becoming a plain
load and store, so that a program which one day has threads has somewhere to put
real atomicity. The time base is a counter, and what it counts at is the
environment's decision. None of them exist.

## How a vector lane is reached

A vector register is 128 bits the instruction set reads as bytes, halfwords,
words, doublewords, or floats, and the guest reads every one of those big end
first. On a host of the other byte order there is no layout in memory that makes
all of those views right at once. Keeping the guest's bytes means a word read
directly comes back reversed; keeping each lane in host order means the bytes
come back in the wrong lane order.

The bytes are kept, which is the same decision guest memory already takes, and
every lane is assembled by an accessor the interface states:

```c
static inline uint32_t xenolith_vector_u32(const xenolith_vector *v, unsigned lane);
static inline void xenolith_vector_set_u32(xenolith_vector *v, unsigned lane, uint32_t value);
```

The register type is a byte array and nothing else. A union holding a word array
beside it would invite reading the word array, which is exactly the mistake the
accessors exist to prevent, and it would pass every test on the machine it was
written on.

Every lane operation builds its result in a temporary and copies it over the
destination at the end. That is not something to optimize away: an instruction
may name its destination as one of its sources, and a merge or a permute reads
lanes the loop has already passed. Building the result elsewhere makes the
aliasing impossible rather than making it depend on the order the lanes happen
to be visited in.

## What is approximated, and why

The rule is that an instruction with no semantics is admitted rather than
approximated. It is kept for anything that computes a value, where a wrong
answer is invisible. It is set aside in exactly two places, both written into
the interface beside the declaration rather than left to be found.

**The machine state register is storage.** Emitted code has no interrupts to
mask, so writing it changes nothing and reading it returns what was last
written. Every use of it in either title is the save and restore pair around a
reservation, where the round trip is consistent and the masking is what is being
skipped. Refusing it would have cost 273 functions over an effect that cannot
exist in the emitted program.

**The vector estimates give more precision than the hardware does.** The
reciprocal, reciprocal square root, exponent, and logarithm instructions are
estimates: the console produces a few significant bits and this produces all of
them. Two implementations of an estimate are allowed to differ, so the
differential cannot compare them exactly, and it does not pretend to.

**The floating point status register is storage on the same terms.** A rounding
mode written into it does not change how a later operation rounds, because the
emitted arithmetic is the host's, in the host's default mode, which is the mode
this processor starts in. An exception enable written into it arms nothing.

A program that depends on either effect will compile and be wrong, and nothing
downstream can tell. That is the cost, and it is why this section exists.

## How this is checked

**Against an emitted corpus.** 1.19 million instructions another project emitted
for the same title, compared on which registers each one reads and writes. That
found nine model bugs.

**Against hardware.** The same encoding is executed on emulated PowerPC with
seeded registers and scratch memory, and run through the C emitted for it, and
the architectural state afterwards is compared. That found seven more, described
on [the verification page](/verification), the last of them by not finishing.

**That it compiles.** Every function of both titles is emitted and compiled with
`-Wall -Wextra`, with no warnings. Compiling only a sample used to be enough
until a function with no blocks emitted a `goto` to a label never written, which
only appeared once everything was built.
