# How it is checked

Nothing here is trusted because it looks right. Each layer is compared against
something produced independently, and each oracle is kept because there is
something only it can see.

| layer | checked against | what only it catches | page |
|---|---|---|---|
| container | two retail titles, plus a reference image from another implementation | decryption and reconstruction together, against ground truth | [container](/verification/container) |
| decoder, mnemonics | `llvm-mc` | an instruction named as the wrong one | [decoder](/verification/decoder) |
| decoder, operands | GNU `objdump`, by value and in order | a field read from the wrong bits, or printed in the wrong place | [decoder](/verification/decoder) |
| analysis | jump tables and helper addresses worked out by hand elsewhere | a function or target that should have been found | [functions](/verification/functions) |
| instruction model | 4.2 million instructions another project emitted, over both titles | an instruction touching the wrong register | [model](/verification/model) |
| semantics | PowerPC executed under emulation | an instruction computing the wrong value | [model](/verification/model) |
| whole functions | functions out of a shipped title, executed under emulation | control flow this project would never have written | [model](/verification/model) |
| the runtime | `tests/runtime.rs`, which compiles the runtime beside a program that drives it | a service that answers wrongly while the emitted code still links | [runtime](/verification/runtime) |

## Running them

```sh
scripts/check           # what needs nothing a machine may lack
scripts/check-oracles   # what needs a title, a toolchain or an emulator
```

`scripts/check` fails the build on any gate it runs. `scripts/check-oracles` runs
the rest with the documented corpus and fails when a test reports itself ignored
rather than run.

Two also run inside `scripts/check` where the machine has what they need: the
differential against hardware when a cross toolchain and an emulator both
answer, and the number check when the corpus is present.

## What a run with no data says

The oracles that need a title, a cross toolchain or an emulator skip when their
paths are unset, so a default `cargo test` passes with nothing installed. That is
why `scripts/check-oracles` treats an ignored test as a failure.

A host C compiler is different, and is required. Most of this suite checks the
emitted C by compiling it, and a suite that skipped those would be comparing
strings.

## Property tests and fuzzing

Property tests assert things that have to hold for any input: truncating a
container at every length yields an error and never a panic; a read of the
address space either returns the length asked for or fails.

Fuzz targets cover the loader, the decoder, the analysis, and the lifter. A fuzz
target on the analysis checks the reachability of resolved flow, so a recovered
jump table cannot introduce a target nothing branches to.

## What none of them reach

The console's vector extension has no execution oracle: no assembler accepts
VMX128 and no emulator implements it. Those instructions have the emitted corpus
and careful reading behind them, and nothing more.

Each oracle page states its own limit too, where a reader of that page will find
it.
