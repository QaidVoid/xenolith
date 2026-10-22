# The decoder oracles

Two comparisons over the same code section, each catching something the other
cannot.

**What only they catch.** Naming an instruction correctly says nothing about
whether its fields were read correctly. A field printed as the wrong kind of
thing, where the number happens to coincide, is invisible to a comparison of
values alone.

**Input.** A code section out of a shipped title, decoded into distinct words, so
the sample is what compilers actually emit rather than a uniform sweep of the
encoding space.

**Commands.**

```sh
XENOLITH_PPC_BINUTILS=powerpc64-linux-gnu- \
XENOLITH_PPC_CODE=.../pes6-recomp/xex_image.raw \
  cargo test --release -p xenolith-ppc operands -- --include-ignored
```

## Values

GNU `objdump` decodes each word and this crate decodes the same word, and the two
operand lists are compared as values. This catches a field read from the wrong
bits.

## Order

The same two lists compared in the order each names them. Order is a second
signal: two disassemblers that agree on the mnemonic should agree on the order
the operands are written in, and one that prints them differently has folded
something into the name or read a field as the wrong kind of thing.

The comparison is restricted to encodings where both sides chose the same
mnemonic, because a different name means one of them folded an operand into it
and the lists legitimately differ. A name that differs only in the letters a
variant bit adds is the exception: that is this crate failing to print the bit,
and it is compared.

**Limit.** The console's own vector forms are excluded from the value comparison.
`objdump` decodes for a target that has instructions this console does not, and
reads several of them as those instead. Two readings that do not name the same
instruction have nothing to say about each other's operands.

The mnemonics comparison runs separately against `llvm-mc`, over both titles, and
around thirteen thousand distinct encodings each way per title agree.
