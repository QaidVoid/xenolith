# The container oracle

**What only it catches.** Decryption and reconstruction together, against ground
truth. A synthetic fixture proves the parser handles the layout someone thought
to write. Only a shipped file proves that layout is the one retail titles use.

**Input.** Two retail XEX containers, read where they already are.

**Pass condition.** The header parses, the image decodes, and the result matches
a reference image another implementation produced from the same file. Every
import record is read, and a record that does not agree with the library that
lists it is rejected rather than interpreted.

**Command.**

```sh
XENOLITH_TEST_XEX=.../ac6_extract/default.xex \
XENOLITH_TEST_IMAGE=.../ac6_lifted/image.bin \
  cargo test --release -p xenolith-xex --test real_container -- --include-ignored
```

**What it is checking.** A container is compressed in blocks and encrypted with a
key that is not in it. Getting either wrong produces bytes that look like data
and decode to plausible instructions, so nothing downstream can tell. The
comparison against a reference image is the only thing that says the bytes are
the ones the console would have loaded.

**Limit.** It needs the key material for an encrypted container, which is why it
is ignored without it. The reference image comes from another implementation,
so a bug both share would agree with itself; that is the reason the instruction
oracles exist as well.
