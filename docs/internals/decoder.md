# The decoder

`xenolith-ppc` turns a 32-bit word into an instruction. It covers 440
instructions across three overlapping instruction sets: 64-bit PowerPC, the
AltiVec vector extension, and VMX128, which is the console's own extension of
AltiVec.

## Encoding forms

An instruction's form says where its fields are. The forms carried here are D,
DS, I, B, SC, XL, A, M, MD, MDS, X, XO, XS, VX, VC, and VA, plus the VMX128
forms the console added.

The form is what makes field extraction a lookup rather than a special case per
instruction, and it matters more than it sounds. The rotate and logical forms
name their destination in the field every other form uses for a source, so an
emitter that read the destination positionally would be wrong for a whole family
and right everywhere else, which is the kind of mistake that survives review.

## VMX128

The console's extension reaches 128 vector registers where AltiVec has 32, and
the extra bits are split across the encoding word rather than sitting next to
the field they extend. Instructions the extension adds and instructions it
merely widens both appear, so which field holds the register number depends on
the form.

No assembler accepts VMX128 and no emulator implements it. It is decoded here
from the public record of the encodings, and it is the part of the decoder with
the weakest independent check, which the verification page says outright.

## An unknown word says so

A word that does not decode is reported as unknown and rendered as `.long` with
the raw value. It is never rendered as the nearest thing that would have
decoded.

This matters more than it seems. A recompiler that guesses at one instruction in
a thousand produces a program that runs and is wrong somewhere nobody will find.
A recompiler that refuses produces a number you can act on.

```
0x823d75bc  0101011d ? .long 0x0101011d
```

## Text output

Rendered text is deliberately close to the encoding rather than to what an
assembler would print. Extended mnemonics are not synthesized, so `li r3, 5`
appears as the `addi` it is, and a branch prints its condition operands as
numbers.

This is a decoder for a recompiler rather than a disassembler for a reader. The
question it has to answer is what the fields hold, and folding fields into an
extended mnemonic destroys exactly that.

## How this is checked

Two independent oracles, each catching what the other cannot.

**`llvm-mc` for the mnemonic.** Every distinct encoding in both titles' code
sections is assembled and disassembled by LLVM and compared against what this
decoder names it.

**GNU `objdump` for the values.** Naming an instruction correctly says nothing
about whether its fields were read correctly. This comparison extracts the
numbers each side printed and compares them as a sorted multiset, over every
distinct encoding in both titles.

That comparison found eight rendering bugs. It has a documented blind spot: a
field printed as the wrong kind of thing, where the number happens to coincide,
is invisible to it. `srawi r4, r3, r4`, which printed a shift amount as a
register, was found by reading rather than by the comparison.

Fuzz targets cover the decoder, so no word can panic it.
