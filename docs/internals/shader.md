# Reading a shader

A shader arrives as words, not as a file. A title hands one to the graphics
processor inside the commands it writes, saying which of two kinds it is and how
many words follow. What `xenolith-shader` reads is a run of words whose length is
known and whose meaning is not.

## Two shapes in one run

A control flow instruction is forty eight bits, so two of them share three
words. Each one says which stretch of the run to execute. The others are ninety
six bits, three words each, and do arithmetic or fetch something.

Which part of the run holds which is not marked anywhere. The control flow comes
first and says where the rest begins, so a run is read by reading the flow and
taking the flow at its word.

## What the decode layer reads

`flow.rs` reads a control flow instruction: which operation, where the stretch
it starts begins and ends, and which slots in that stretch fetch rather than
compute. `alu.rs` reads an arithmetic instruction into its two halves, one
across four lanes and one in a single lane. `fetch.rs` reads what a fetch goes
and gets: a vertex or a texture, the slot it fills, and how many numbers it
brings back.

`find.rs` is the other direction. Given an image it reports every run that reads
as a shader, at the word it starts on. It is how a shader handed over at runtime
is matched to one found in the image beforehand.

## Decoding never fails

A word encoding nothing this crate recognizes reports an unknown operation
rather than being guessed at. A wrong decode becomes silently wrong output; an
unknown one is a loud failure at the time of reading.

Every read goes through a bounds-checked accessor, so a run whose length a title
got wrong stops the reading rather than taking whatever sits after it.

## What makes a reading right

Not that each field looks plausible on its own. It is that the parts agree.
Every stretch of arithmetic a shader says to run begins where the last one
finished, the stretches cover the shader with nothing left over and nothing
twice, and the shader stops on the one operation that means stop. A layout read
wrongly does not do that, which is what `tests/real_shaders.rs` checks against
two shaders taken from a retail title as it ran.
