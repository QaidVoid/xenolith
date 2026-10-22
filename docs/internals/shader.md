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

## How the translation is laid out

The decode layer is one module per thing read. The translation is one module per
thing written, because what it says is not one subject:

- `constants.rs` holds the console's own numbers: registers, constants, what is
  carried on, and where among the constants a title's run time values sit.
- `bindings.rs` makes what a shader is handed from outside: the constant block,
  the window on a title's memory, and the pictures.
- `edges.rs` makes what a shader says at its edges: what it produces, what it
  carries on, how big a point is, and which vertex this is.
- `preamble.rs` calls those in the order a module needs them and hands back the
  names the rest of the translation is written against.
- `operands.rs` reads what an instruction reads. `values.rs` holds the small
  operations everything else is written from. `arithmetic.rs` is the four lane
  half, `single_lane.rs` the single lane half, `fetching.rs` the fetches, and
  `plan.rs` reads a shader ahead of writing it to find what a fetch needs named.
- `dispatch.rs` writes one instruction out and gives the two ways of reaching
  instructions, straight through and jumping. `translate.rs` holds the result
  types and the reasons a shader is refused.

No file in `src` may exceed 700 lines, and `tests/file_size.rs` fails the build
where one does. The translator arrived as a single file of nearly four thousand
lines, and the cap is what stops that happening again.

## Finding one in an image, and naming it

Two functions stand in front of the translation.

`reads_as_shader` looks at a run of words and says whether it begins like a
shader, returning where the shader starts. Padding reads as a shader just as well
as the shader after it does, so without this a run would report one starting from
every word of nothing in front of it, and where it starts is what a title hands
over. Every shader a title was seen to hand over begins with a stretch to run or
with room reserved.

`sum_of` is the name a shader is known by. A title hands over words and not a
name, so the sum of those words is what the runtime looks the translated module
up with. It is a hash over the bytes, and the same words always give the same
sum, which is the whole of what is required of it.

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

## The golden module

Nothing here can say whether a translation draws the right thing, and the
validator that would say whether the module is well formed is not installed on
every machine. So the module one shader becomes is committed as a word list in
`tests/fixtures/pixel_shader.txt`, and a test compares the translator's output
against it word for word.

That is the regression guard. A change that alters one emitted word fails with
the word and its position named. The fixture is also walked as a module, every
instruction stepping by the length it declares, so a copy of a broken module
would not match either.

The fixture was emitted once and read through before being committed. It is not
evidence that the translation is correct, only that it has not changed.
