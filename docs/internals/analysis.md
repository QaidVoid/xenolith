# Finding functions

`xenolith-analysis` takes a decoded image and works out where the functions are,
what basic blocks they are made of, and where control flows between them. This
is the part that a comparable tool asks you to write down by hand.

## Discovery

A worklist run to a fixed point, seeded from everything the image itself says:
the entry point, and every address a call instruction names. Each function
discovered adds its own call targets, until nothing new appears.

That finds everything reachable from the entry point by a direct call, which is
most of a program and not all of it. The rest is found two other ways.

**Prologue scanning.** A function that is only ever reached indirectly still
begins the way every other function does, saving the link register and moving
the stack pointer. Scanning executable memory for that shape finds functions
nothing points at. A seed has to be four byte aligned to be considered at all,
since a byte pattern found at an odd offset is a coincidence rather than a
function.

**Recovered jump table targets.** Covered below.

Discovery and recovery are alternated to a fixed point rather than run once
each. A recovered table names blocks that were never walked, walking them finds
calls to functions never discovered, and those functions have tables of their
own. Alternating raised coverage on one title from 75.8% to 88.0%, and feeding
recovered targets back into discovery as additional entry points took it to
95.3%.

## Register save and restore helpers

The compiler that built these games emits calls into a set of shared helpers
that save and restore ranges of registers. There are eight of them, and the
comparable tool asks you for all eight addresses.

They are detected here by their structure. A restore helper is a run of loads at
descending offsets from the stack pointer, each into the next register down,
ending in a return. Nothing about that shape depends on which game it is in.

All eight are found on both test titles without being given any of them, and the
addresses match values two other projects recorded by hand.

## Jump tables

A `switch` compiles to a load from a table of offsets followed by a branch
through a register. Recovering it means reconstructing, from the instructions
before the branch, where the table is, how wide its entries are, and how many
there are.

This is a forward abstract interpretation over the predecessor chain. Each
register holds one of: unknown, a constant, an entry loaded from a table, or a
base plus such an entry. The bound comes from the comparison that guards the
branch.

Several things make it harder than it sounds.

- **Entries are scaled after loading.** A `rlwinm` that shifts a loaded entry is
  part of the address computation, and missing it made every recovery fail.
- **The index is scaled before use.** Tracking which register descends from which
  index, and by what multiplier, is needed to connect the bound to the table.
- **A table is not always in the section you are reading.** Tables live in read
  only data while the branch is in code.

A table found in writable memory is rejected. A program can write to it, so its
contents at run time are not what they are in the file, and emitting a dispatch
over what the file happens to hold would be a guess.

803 of one title's 852 tables are recovered and agree exactly with what the
comparable tool produced, with zero disagreements. The remainder are reported as
unresolved, and lifting emits a call through the runtime's indirect dispatch for
them rather than inventing targets.

## What it reports

```sh
xenolith analyze default.xex --functions --tables
```

The report gives how many functions were discovered and how each was found, what
proportion of executable words are claimed by a function, and how many indirect
branches were resolved against how many were not. Resolved and unresolved
branches are counted apart, because a tool that folds them together can improve
one number by getting worse at the other.

## How this is checked

- Helper addresses and jump tables are compared against values worked out by
  hand elsewhere, on both titles.
- An invariant asserts that no block is ever claimed without an edge reaching
  it, so a block cannot be attributed to a function that cannot get to it.
- A fuzz target checks the reachability of resolved flow, so a recovered table
  cannot introduce a target nothing branches to.
