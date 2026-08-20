# How it is checked

Nothing here is trusted because it looks right. Every layer is compared against
something produced independently, and each oracle is kept because there is
something only it can see.

## The oracles

| layer | checked against | what only it catches |
|---|---|---|
| container | two retail titles, plus a reference image from another implementation | decryption and reconstruction together, against ground truth |
| decoder, mnemonics | `llvm-mc` | an instruction named as the wrong one |
| decoder, operands | GNU `objdump`, by value and in order | a field read from the wrong bits, or printed in the wrong place |
| analysis | jump tables and helper addresses worked out by hand elsewhere | a function or target that should have been found |
| instruction model | 4.2 million instructions another project emitted, over both titles | an instruction touching the wrong register |
| semantics | PowerPC executed under emulation | an instruction computing the wrong value |
| whole functions | functions out of a shipped title, executed under emulation | control flow this project would never have written |

## What each one found

**GNU objdump found eight rendering bugs.** Naming an instruction correctly says
nothing about whether its fields were read correctly, and the mnemonic
comparison could not see any of them. Condition register operands, unsigned
logical immediates, branch condition fields, and float operands were all being
printed from the wrong bits.

That comparison had a blind spot worth stating: a field printed as the wrong
kind of thing, where the number coincides, is invisible to a comparison of
values. `srawi r4, r3, r4` printed a shift amount as a register and was found by
reading, not by the oracle. Comparing the text closes this one, since `v4` and
`4` differ as text however much their numbers agree, and it duly turned up a
vector shift printing its byte count as a register.

**It had a second blind spot, and comparing in order found 56 more.** The values
were compared sorted, because this project printed operands in encoding order
and an assembler prints them in the order it accepts. Sorting made the two
comparable and made operand order invisible.

Everything it hid was real. The logical operations and the shifts print their
target where the encoding stores a source, so `slw r8, r11, r11` was an
instruction that writes r11 and reads r8, and nothing in that text said which
was which. The float and vector loads named their register in the general
purpose bank, printing `r0` for `f0`. An address built from a base and an index
reads register zero as the number zero, and printing `r0` claimed a register was
read that is not. A fused vector multiply named its operands in the wrong two
slots. A special purpose register move printed the two halves of the register
number as two registers. Every one of them was a sentence about the instruction
that was not true.

So the ordering comparison is now permanent, restricted to encodings where
objdump chose the same mnemonic. Agreeing on the name means neither side folded
an operand into it, and two disassemblers naming the same instruction should
name its operands in the same order. It reaches slightly more than the sorted
one, since it needs no list of extended spellings to exclude.

Both run over both titles: 942,332 instructions, no disagreements.

**The emitted corpus found nine model bugs, and later two more.** It compares
which registers each instruction reads and writes, over 2.14 million of them for
one title and 2.12 million for the other, which reaches families the execution
differential is far too slow to touch.

Run against the second title it found nothing new, which after three rounds of
the opposite is worth recording. The ten disagreements left there are both
defects in that output rather than in the model: the recording vector comparison
below, and a vertex unpack it refuses to emit at all.

For a long time it reached none of the vector families at all. The corpus writes
a vector through an intrinsic rather than an assignment, so the parser recorded
every vector destination as a read and no writes at all, and the comparison was
vacuous for exactly the instructions with the weakest other evidence. Once it
could read them it immediately disagreed about the console's forms, and it was
right: the first source was being read from a bit the opcode owns.

It has also been wrong once. It emits a recording vector comparison without ever
setting the condition field, then reads that field in the next instruction. That
is a defect in the oracle rather than in the model, recorded rather than matched.

**Execution found six real semantic mistakes on its first run.** The worst was
that the record bit compared the low 32 bits of a result rather than all 64, so
every dot suffixed instruction set the wrong condition whenever a result's
halves disagreed in sign. Nothing else built could have seen it: the corpus says
which registers are touched, objdump says which fields exist, and neither can
say that a comparison used the wrong width.

**It found two in the vector families.** The fused multiply and add rounds once
between the two operations rather than twice, and writing it as a multiply
followed by an add is one place out for some inputs. Four runs of one
instruction disagreed, out of a differential over sixty six vector instructions,
and no amount of reading would have found it.

**It found a seventh by hanging.** A conditional store's low encoding bit
is part of its spelling rather than a record bit, and the emitter read it as one.
The generic path appended a comparison of the stored value against zero, which
overwrote the condition field the store had just set from its own outcome. The
retry branch after it then read a bit that always said the store had failed, and
the emitted code looped forever.

Reading that code would have shown two writes to the same field and looked like
harmless duplication. The corpus would have agreed with it, since the touched set
was right either way. Only running it showed the loop, and it showed it by not
finishing, which is a symptom worth taking as seriously as a disagreement.

**Running real functions found three more, all in control flow.** Once the
emitted C linked, whole functions out of a shipped title could be run on both
sides. The first sample of four hundred disagreed about eleven of them, and the
eleven came down to three defects.

A conditional branch to the link register was emitted as an unconditional
return. Every conditional return in every title returned always, which is to say
the first one a function reached ended it. The same mistake covered a
conditional branch to the count register and a conditional call.

The branch forms that decrement the count register never decremented it and
ignored the count entirely, so every counted loop in the output was infinite.

And `addic.` records its result by virtue of which opcode it is rather than by a
record bit, and the emitter only looked at the bit. It never wrote the field, so
a countdown loop tested a condition its own subtraction was supposed to have
set, and never saw it change.

None of the three was reachable from the older oracles, and it is worth being
precise about why. The corpus compares which registers each instruction touches,
and the effect model already recorded that `addic.` writes a condition field and
that a counted branch reads and writes the count. Both were right. Only the
emitter was wrong, and the corpus oracle never looks at the emitter. The
execution differential runs one instruction, or a short sequence written here. A
conditional return needs a function to return from and a counted loop needs a
loop, and neither existed to be run.

**Pointed at the second title, it found three more.** It had only ever run
against one, which in hindsight was the mistake: one pass over the other
disagreed about twelve runs on its first try.

Two were defects in operations that look like they work on thirty two bits and
do not. A word rotate whose mask ends before it begins wraps past the last bit
and selects the whole high half of the register, so it writes both halves rather
than zero extending, and an insert leaves the high half alone when its mask does
not wrap. Every rotate this project emitted zero extended.

The third was the carry. The add and subtract family sums a pair and a carry,
three terms, and the carry out was read from a single comparison of the result
against an operand. That test only holds for two terms: subtracting a value from
itself gives all ones, adding the carry wraps it to zero, and no comparison of
that zero against either operand says a carry happened.

What is worth recording is why the existing subjects could not have found them.
The subject list already had a rotate named for the wrapped case, and its mask
ran from four to twenty, which does not wrap. Every seed row started the
destination register at zero, so an instruction that reads what it writes was
forever being checked against an already empty destination, and clearing the
high half looked the same as preserving it. Both have been corrected, along with
the rest of the carry family and a pair of sequences that carry from one
instruction into the next, and each new subject was checked to fail when its fix
is reverted.

A third disagreement was the harness rather than the model. The differential
seeded no stack pointer, so a function that spilled to its stack and read back
was comparing the hardware's own stack against guest address zero. Both sides
are now given the same swept guest stack.

**Sampling five times as deep found three more still.** The sample was four
hundred functions; at two thousand it disagreed again, and stays there. Five
times the coverage for three times the wall clock is a good trade for an oracle
that only runs against a real title.

The doubleword clear and insert took their mask to run to the last bit, where it
ends wherever the shift left off, so an insert overwrote everything the shift had
moved past rather than leaving it. The scalar fused multiplies were written as a
multiply and then an add, rounding twice where the architecture rounds once. The
vector form of that was fixed when the vector differential caught it, and nobody
went back to look at the scalar one.

The last was two more unseeded registers, the same gap as the stack pointer: a
stack probe read a scratch register the wrapper had left something in while the
model saw the zero it starts from. Everything the calling convention lets a
function use without saving is now cleared on both sides.

**The scalar floating point family had no execution oracle at all.** The
instruction differential seeded general registers, condition fields, the
exception register, memory, and vectors, and never a floating point register, so
the whole scalar family rested on the corpus and on reading. That was not a
hypothetical gap: it is why the fused multiply above survived until a real title
exercised one.

It is closed. Ten floating point registers are seeded and compared, as bits
rather than as numbers so that the two zeroes stay distinct and a result which
is not a number has to match in the payload it carries. Twenty seven subjects
cover both precisions: the arithmetic, the fused multiplies, the moves and sign
changes, the narrowing and the conversions, the select, and both compares.

The seeds are the point. Two of them are a pair of values and the value their
product rounds to, one such triple at each precision. A fused multiply and add
over a triple is exactly the error the rounding threw away, and the same thing
written as a multiply and then an add computes it as zero. Both products are
also present negated, so the forms that add cancel the way the ones that
subtract do. Reverting the fix makes all eight fused forms disagree on every
seed, which is the check that the subjects are worth having.

**Widening it to call graphs found a bad one.** It had been refusing any
function that called anything and any function holding a recovered jump table,
which over one title's emitted C is 19,310 and 659 functions of 27,276. Three
things had therefore never been executed at all: the call and return path, the
eight register save and restore helpers, and every recovered jump table.

The first pass over the wider set found that a conditional trap was emitted as
an unconditional one. Compilers put a trap after a division to catch a zero
divisor, so every one of those stopped the emitted code on the check rather than
on the fault it was checking for. Nothing before could have reached it: a trap
sits after a division inside a function that calls, and the instruction
differential's stub for a trap did nothing, so a trap fired there changed no
state to compare. The stub now ends the run, and four traps whose conditions
never hold are checked to fail when the fix is reverted.

Following calls needed a decision about what to do at the edges. An entry brings
everything it calls with it, transitively and bounded, so the model has a
function to enter. A computed target is looked up among those, so a recovered
table lands on one. A target that resolves to nothing is reported rather than
skipped, which is the point: it means the table behind it was not fully
recovered, and that recovery has until now only ever been compared against
another project's tables.

Two things are skipped rather than compared. An import nothing implements, since
the environment is unimplemented by design. And anything reading the time base,
which is a clock the two sides read differently, the same reason the instruction
differential leaves it out.

**Following calls needed the register helpers, and that is what made a title
run at all.** A call into a save or restore helper lands partway through it,
depending on how many registers the caller wants kept. Discovery does not claim
those addresses, and rightly: a call to a helper is a call to the helper, not to
a function beginning wherever the caller entered. But emitted code still needs
something to call, and without a body each one became a trap.

Better than a third of a title's functions save registers that way. Running one
from its entry point stopped on its first instruction, at a helper that was not
there. Lifting each entry a caller uses turned that into a program that reaches
its first call into the operating system.

It widened this oracle at the same time, because an entry whose callee could not
be lifted was refused outright. Eligible functions went from 2,810 to 3,556, and
the ones holding a call from 737 to 1,483.

**Two thirds of the jump table failures were never jump tables.** Recovery
reported 130 branches it could not read. Disassembling them showed 87 reaching
their target through two loads, one for a table held in an object and one for a
method inside it, which is how a virtual call is compiled. There is no table
behind one and there never will be. Counting them against the recovery made it
look worse than it was and hid the ones that really were missed. They are now
reported as what they are, which leaves 57 branches whose tables were not read.

The rest were a bound, not a table. Recovery followed one predecessor and gave
up at the first fork, so 37 tables whose addresses it had fully tracked never
saw the range check guarding them. It now walks every path into the dispatch and
requires the table and the bound to agree on all of them, which is sound:
whichever way control arrived, the check held. That took the reference agreement
from 803 of 852 to 809, with no disagreements either way.

Worth stating plainly: that last part improved the measurement and not the
output. The emitted C is unchanged, to the instruction, because the analysis
already reached those tables by walking again with more boundaries known. What
was wrong was the recovery reported on its own, which understated what the
analysis achieves.

**A prologue is longer than four instructions.** Scanning for functions nothing
calls looked at four instructions for the signs of a prologue: reading out the
link register, calling a helper to save registers, and moving the stack pointer
down. A function that saves only two or three registers writes them inline
rather than calling a helper, which pushes the stack adjustment past the fourth
instruction, so it scored one signal where two were wanted and was never found.

Widening the window to eight found 2,572 more functions in one title and 5,206
in the other, taking claimed code from 87 to 93 percent and from 95 to 98. They
lift with no blocker that was not already there, which is the first sign they
are real: data misread as code hits a word that does not decode. The differential
then ran 238 of them against hardware over 899 runs with nothing disagreeing,
which is the second.

It compounds with the tables. More functions means more blocks to read a
dispatch out of, and jump table agreement with the hand written reference went
from 809 of 852 to 822, still with nothing disagreeing.

Widening it also exposed one more unseeded register, this time a floating point
one: a compare against a register the harness never set read a leftover on the
hardware side and a zero in the model. The volatile floating point registers are
cleared on both sides now, the same as the general purpose ones.

**How far the unclaimed words can go, measured rather than guessed.** After the
wider prologue search one title still had seven percent of its executable words
claimed by nothing. Three things were tried and only one was worth keeping.

Widening the window further, from eight to twelve, found 215 more functions
against the 2,572 the first widening found. Diminishing, and taken no further.

Counting a register saved below the stack pointer as a sign of a prologue found
800 more functions and claimed 133 more words. That is not discovery, it is
fragmentation: a new boundary shortens the function that had covered the region,
so the coverage moves rather than grows. Rejected.

Then the shortfall itself was measured. A fifth of it does not decode at all,
which is data. Another fifth is words that are addresses into the code section,
which is what a jump table is made of, and every entry in one decodes as some
instruction or other, so a run reading 99 percent decodable is not therefore
code. What is left has no prologue: leaf functions short enough to keep their
frame in the red zone, which read out no link register and move no stack
pointer, and so leave nothing to recognize.

The last thing tried settles it. Every word in the whole image was checked for
being an address that begins like a function and that nothing claimed. There are
none, in either title. Discovery already reaches everything the file points at,
and what remains would have to be found by guessing. That check is kept as a
test, because a function the image names and nothing claims is a real defect and
there is no reason for one to exist.

## The execution differential

This is the strongest oracle for a single instruction, so it is worth
describing.

An instruction encoding is placed as a word in an assembly file the harness
writes, with every register seeded by name and every register that could have
changed stored afterwards, along with the condition and exception registers and
a window of scratch memory. That is built with a big endian ppc64 cross
toolchain and run under a user mode emulator. The same encoding is run through
the C this project emits for it, from the same state, on the host. The two are
compared.

Several decisions in it were learned the hard way.

**Registers are named in assembly the harness writes.** The first prototype used
inline assembly with register constraints, and the compiler allocated its own
temporaries over the seeded registers. It reported that adding two numbers
returned one of them.

**The instruction is placed as a word, not assembled from text.** Assembling
rendered text would test the renderer rather than the decoder, and would fail for
the families this project deliberately prints in encoding order.

**Several inputs per instruction.** A single input pair agrees by accident too
often. Zero and zero agree for nearly every operation, and two small positive
numbers hide every sign and overflow question. The seeds include negatives,
values whose low word disagrees with their high, and values that carry out.

**Scratch memory is seeded with a pattern rather than zeroes.** A load of the
wrong width or byte order brings back something visibly wrong from a pattern and
something indistinguishable from a buffer of zeroes.

**Control flow is checked by running whole functions.** A branch's effect is on
which instruction runs next, which no single instruction's registers can show,
and the code for one is written by the emitter from the graph rather than by the
model from the encoding. Short functions are run on hardware and lifted and run
on the host, and the harness checks that the outcomes actually differ across
inputs, because two sides can agree while neither took a path.

**Only the modelled bits of the exception register are compared.** The emulator
reports bits this project does not model and lifted code never reads.

## The real function differential

The execution differential runs sequences this project wrote. That is enough to
check one instruction and not enough to check a function: the sequences are
short, whoever was testing an instruction chose them, and the control flow in
them is whatever the test needed.

A shipped title has thirty thousand functions nobody here wrote. This oracle
takes leaf ones out of it, seeds a state, runs the address under emulation
against the real image and the emitted C on the host, and compares the volatile
registers, the condition register, the modelled exception bits, and a window of
memory.

**The functions are spread across the whole code section.** The first sample was
the first sixty four found, all inside one neighbourhood, and it found nothing.
Functions near each other were written together and do the same kinds of thing.
Four hundred taken at an even stride from end to end found three defects at once.

**Both sides have to see the same memory.** The model reaches memory as a base
plus a truncated address, over four gigabytes it always has. The hardware side
has whatever it mapped, and a real function computes addresses from what it was
handed rather than from what a harness expected it to touch. The driver is
linked at sixteen gigabytes, out of the guest address space entirely, so the
whole low four gigabytes is free to be mapped the way the model maps it. The
first attempt put the scratch where a big endian ppc64 program loads itself and
replaced the running program's own code.

**A function is tried under several shapes of starting state.** It does not say
which of its arguments are pointers and which are counts, and the two want
opposite seeds: give a count a pointer and the function scales it out of the
mapping, give a pointer a count and it dereferences nothing. Each was tried
alone and each cost five of every six functions.

**A run that did not finish on one side is skipped and counted as skipped.** A
comparison needs both sides to have finished, and a harness that quietly dropped
those would report a coverage figure that meant nothing.

## What this cannot reach

**The console's vector extension has no execution oracle.** No assembler accepts
VMX128 and no emulator implements it. Those instructions have the emitted corpus
and careful reading behind them, and nothing more. A coverage figure that folded
them in would imply more than it means.

Its permute forms are refused outright rather than modelled. Their control
fields sit in bits this project has no independent reading of, and a permute
built from the wrong bits produces a plausible vector rather than an obvious
mistake, which is the worst kind of thing to guess at.

**The emulator is a generic POWER, not a Xenon.** Where the two differ, the
harness would report the model wrong when it is right. That is a real limit and
the reason a disagreement is read before it is believed. Two of the first three
disagreements found were the harness and the test data rather than the model.

Some instructions are excluded for that reason. `mfmsr` and `mtmsrd` are
privileged and a user mode emulator will not run them. `mftb` returns a clock,
so there is nothing to compare. `dcbz` operates on a block size that is
implementation defined, and the emulator's is not this console's, so its size is
taken from the documentation and checked against nothing.

**Coverage is a third of the instruction set, deeply.** The differential
executes a few hundred encodings on several inputs each. The corpus covers a
million instructions shallowly. Neither replaces the other.

**Most real functions cannot be run under emulation at all.** The console runs
its titles in the thirty two bit mode, where an effective address is truncated
before it is used, and the model does the same. `qemu-ppc64` runs in the sixty
four bit mode and truncates nothing, so an address a title formed by sign
extending something at or above two gigabytes arrives with its top thirty two
bits set. That covers most of the image's own globals.

Mapping those pages a second time where the emulator looks for them would fix
it, and that address is above the host's own user address limit, so it cannot be
done. The function faults on the hardware side and the run is skipped. About
three quarters of a sample is lost this way, which is why the sample is large
rather than why the result is weak: what does run is compared exactly.

## Property tests and fuzzing

Property tests assert things that have to hold for any input: truncating a
container at every length yields an error and never a panic; a read of the
address space either returns the length asked for or fails.

Fuzz targets cover the loader, the decoder, the analysis, and the lifter. A fuzz
target on the analysis checks the reachability of resolved flow, so a recovered
jump table cannot introduce a target nothing branches to.

## Running the real oracles

The suite passes on a clean checkout with no game data present, which is a
weaker statement than it looks. Tests needing a real title, a cross toolchain, or
an emulator read paths from the environment and skip when they are unset.

[Environment](/reference/environment) lists every variable and what it names.
Run those with `--release`; in a debug build they take minutes.
