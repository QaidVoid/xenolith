# The runtime interface

Every function `xenolith.h` declares, grouped by what it is for. The emitted C
is written against this header and nothing else; [The runtime](/internals/runtime)
describes what sits behind it.

`scripts/check-docs` fails the gate when the header declares a function this
page does not name.

## Guest memory

Every guest access goes through these. They assemble values from bytes in the guest's big endian order, so emitted code does not depend on the host's byte order.

### `xenolith_load8`

Reads a byte of guest memory.

### `xenolith_load16`

Reads two bytes of guest memory in the guest's big endian order.

### `xenolith_load32`

Reads four bytes of guest memory in the guest's big endian order.

### `xenolith_load64`

Reads eight bytes of guest memory in the guest's big endian order.

### `xenolith_store8`

Writes a byte of guest memory.

### `xenolith_store16`

Writes two bytes of guest memory in the guest's byte order.

### `xenolith_store32`

Writes four bytes of guest memory in the guest's byte order.

### `xenolith_store64`

Writes eight bytes of guest memory in the guest's byte order.

## Numbers

Conversions and arithmetic where the console's answer differs from what a C cast or operator would give.

### `xenolith_to_a_whole_number`

Converting a single precision value to a whole number, with saturation.

The instruction set clamps rather than wrapping, and a C cast does neither:
a value that is not a number gives the console's one answer for that too, and
a C cast leaves it to the host. Both are stated here so that every host gives
the console's answer rather than its own.

The rounding mode is not modelled: the header says so at the floating point
status register. Both `nearbyint` and `llrint` honour the mode, so this holds
only because nothing here changes it and the host starts at round to nearest
with ties to even. A program that called `fesetround` would move these.

Rounding happens before the range is tested, because a value just inside the
range can round to just outside it: 2147483647.5 is in range and rounds up to
2147483648, which a cast cannot represent.

### `xenolith_to_a_whole_word`

The same for the form that gives a 32-bit whole number.

The result is sign extended into the 64-bit register. The instruction does not
define what the upper word holds; sign extension is this implementation's
choice, taken so that the overflow path and the ordinary path agree. The value
out of range is the 32-bit one the console gives, not the 64-bit one: a 32-bit
conversion that overflows has nowhere to put a 64-bit answer.

### `xenolith_to_a_whole_number_by_cutting`

The form that cuts the fraction off rather than rounding.

A cast does exactly this, so the cast is the whole of it. What a cast does not
do is answer for a value outside the range, and that is the same as above: a
value that is not a number, and a value too large, each have the console's one
answer and this states it rather than leaving it to the host.

### `xenolith_to_a_whole_word_by_cutting`

The 32-bit form of the one above.

### `xenolith_half_to_float`

Turns one of the console's half sized floats into a whole one.

It is not quite the one everybody else keeps: there is no value that stands
for infinity and none that stands for not a number, so the largest exponent
is just the largest exponent, and a value too small to be written with an
exponent of its own is nought rather than being written the long way. Both
follow from the shift being done as it stands, which is why this is written
out rather than handed to a host instruction that would round it the other
way.

### `xenolith_saturate_signed`

Converting a single precision value to a fixed point one, with saturation.

The instruction set clamps rather than wrapping, and a value that is not a
number becomes zero. Writing a cast instead would be undefined for exactly
the inputs this has to get right.

### `xenolith_saturate_unsigned`

The unsigned form of the one above.

### `xenolith_multiply_high`

The high half of a doubleword product.

C has no type wider than the operands to hold it in, so it is built from the
word products the way it would be done by hand. A compiler that has a
double-width type of its own recognizes this and emits the single
instruction; one that does not still gets the right answer.

### `xenolith_multiply_high_signed`

The signed form of the one above.

### `xenolith_clamp`

Clamping a wider intermediate back into a lane.

The saturating forms of the vector arithmetic stop at the end of the range
rather than wrapping, so each is computed at a width that cannot overflow and
then brought back. A clamp at the wrong bound gives an answer that is almost
right, which is the kind that survives review.

### `xenolith_clamp_unsigned`

The unsigned form of the one above.

## Vector registers

A vector register is kept as its 16 bytes in the guest's order, and every lane is read and written through these accessors.

### `xenolith_vector_u8`

Reads byte lane `lane` of a vector register.

### `xenolith_vector_set_u8`

Writes byte lane `lane` of a vector register.

### `xenolith_vector_u16`

Reads halfword lane `lane` of a vector register.

### `xenolith_vector_set_u16`

Writes halfword lane `lane` of a vector register.

### `xenolith_vector_u32`

Reads word lane `lane` of a vector register.

### `xenolith_vector_set_u32`

Writes word lane `lane` of a vector register.

### `xenolith_vector_u64`

Reads doubleword lane `lane` of a vector register.

### `xenolith_vector_set_u64`

Writes doubleword lane `lane` of a vector register.

### `xenolith_vector_f32`

A float lane moves through its bits rather than through a pointer aimed at
them, because copying is what the language defines and the pointer is what a
compiler is allowed to assume nothing does.

### `xenolith_vector_set_f32`

Writes float lane `lane` of a vector register, through its bits.

### `xenolith_vector_shift_left`

Shifting a whole vector register, across lane boundaries.

The amount is in bits and may be anything up to the width of the register, so
a byte of the result generally takes bits from two bytes of the source. The
shifts that move whole bytes are the same operation with a multiple of eight.

### `xenolith_vector_shift_right`

The same, to the right.

## Vertex packing

The lanes of the console's vertex pack instructions.

### `xenolith_pack_colour`

One lane of a colour a title is packing into a vertex.

The console does not convert the number, it reads the bits underneath it. A
shader hands over a value between three and the next two hundred and
fifty five representable above it, so the byte wanted is already sitting in
the low bits of the float and the whole of the work is deciding whether the
value is in that window. Below it, or not a number at all, gives nothing;
above it gives everything.

Comparing the bits for the upper bound is only sound once the value is known
to be at least three, which the first test establishes: over the positive
floats the bit patterns run in the same order as the values.

### `xenolith_pack_short`

One lane of a pair or a quartet of signed halfwords, read the same way.

The window is the thirty two thousand seven hundred and sixty seven either
side of three, and what falls outside it stops at the end it left by. The
low bound is one above the most negative halfword, which is what the console
saturates to rather than the halfword below it.

### `xenolith_pack_half`

One lane of a pair or a quartet of half precision values.

This one is a conversion rather than a reading of bits, so it rounds, and it
rounds to nearest with ties going to even the way every other narrowing in
this instruction set does. The format has no infinity and no value that is
not a number, so a value too large to hold takes the largest exponent and one
that is not a number keeps a mantissa bit set; neither means what the names
would mean in an ordinary half, and widening either gives a finite float.

## Machine state

### `xenolith_zero_block`

Zeroing a data cache block.

The block is the console's cache line, which is 128 bytes. This console
spells it two ways, and the decoder folds both into one opcode, so both
clear a line. The address is aligned down to the line.

A general purpose emulator does not share the line size, so this is the one
instruction here with no execution oracle behind it.

### `xenolith_condition_pack`

The condition register as one word, and back.

The first field occupies the most significant four bits, and within a field
the order is less than, greater than, equal, summary overflow. Both
directions are written here rather than at each use, because the same bit
order mistake made in both would cancel out and survive.

The mask names fields from the first, so its most significant bit selects
field zero.

### `xenolith_condition_unpack`

The other direction: a word back into the condition fields the mask selects.

### `xenolith_reserve32`

Where a reservation is taken and where it is redeemed.

A program with one thread and no sharing could load and store these directly
and be right, which is exactly why they are not emitted that way: the
assumption would sit where nothing could find it. The runtime implements
these with real atomics, since guest threads are real host threads.

The conditional store reports whether the store happened, which is what the
retry branch after one reads.

### `xenolith_reserve64`

The doubleword form of `xenolith_reserve32`.

### `xenolith_conditional32`

The conditional store that redeems a reservation. Returns whether the store happened.

### `xenolith_conditional64`

The doubleword form of `xenolith_conditional32`.

### `xenolith_timebase`

The time base, which advances.

At what rate is the runtime's decision. Emitting a constant would produce a
program whose timing loops never finish.

## Leaving emitted code

Where control goes when it cannot stay in a lifted function.

### `xenolith_trap`

Where a trap that fired goes.

A trap leaves the function it was in, so what happens next is decided by the
runtime rather than by emitted code.

### `xenolith_break`

A title asking for a debugger, with the number saying why. A trap that always
fires, against register nought and a number, is how a title spells that, and a
console with no debugger attached carries on past it. So does this, after saying
where it was asked for the first time each place asks.

### `xenolith_dispatch`

Where a target unknown when the code was emitted becomes a function.

This is the single place an address is resolved at run time, so that a branch
whose table could not be read has one path rather than many.

### `xenolith_unlifted`

Where a call to a function that was never translated goes.

Told apart from a trap because the two are different events. A trap is an
instruction the title ran on purpose, and this is a function the translation
could not express, so reading one as the other sends a reader looking in the
wrong place.

### `xenolith_import`

Where a call to an imported function goes.

The container names an import by ordinal within a library and never by name,
so the ordinal is what the title actually said. The name beside it comes from
a catalogue of what the console's libraries export and is null where the
catalogue has no entry, so a reader gets something to act on without the
ordinal ever being replaced by a guess at it.

What the ordinal does is decided by the runtime.

### `xenolith_lookup`

Where an address becomes the function lifted for it.

Only the lift knows which addresses it emitted, so the table is written
beside the code rather than here. Anything not in it is not a function this
program has.

## Starting a title

Called by the generated `main.c` before the title is entered.

### `xenolith_boot`

Mapping guest memory and loading the image into it.

Emitted code reaches memory by adding a guest address to a base, so the base
has to be a mapping wide enough that any address it could form indexes
inside it. Returns the base, or a null pointer if the image could not be
read.

### `xenolith_processor_block`

The block a title reaches through r13, which is what it reads the kernel's
tick count and its own processor number out of.

A title that finds nothing there reads processor nought on every thread, and
its vector code picks a slot from that number and steps a counter in it, so
two threads on one block step the same counter and carry it past the end.

### `xenolith_built_against`

Says which kit a title was built against, read out of its container.

The graphics library a title links keeps its device in a layout that changes
between kits, and the runtime answers a few words of that layout: how far the
hardware has got, in the two forms the library keeps. Told the kit, it knows
where those words are; told nothing, it answers neither and a title that
watches them decides the hardware has stopped.

### `xenolith_privileges`

Says what a title's own container declares it is allowed to do.

A title asks whether it holds a numbered privilege, and the answer is a bit
in the system flags word its executable carries. That word is in the
container and nowhere else, so it is passed in rather than guessed at. A
title told it holds nothing takes the path for a build that was not licensed
to do what it does, which on the titles tried so far is to shut itself down
before it draws anything.

Nought means the container did not say, which is answered the same way as a
word with the bit clear: nothing here invents a privilege.

### `xenolith_boot_the_game`

Maps the guest's memory, puts the player's copy of the game in it and
returns its base, or null where it could not.

The game is the folder `XENOLITH_GAME` names, or `games/<id>/` in xenolith's
directory of the user's data, and its `default.xex` is read with the
player's key. A copy whose image does not have `fingerprint` is another
version or region of the game, or another game, and is refused. The folder
is then where the title's files are read from, and it is never written: a
title writes only into what it saves, as it does on a console whose disc
cannot be written.

Reading the executable takes the library a lift copies beside the program,
and a program built without it says so and boots nothing.

### `xenolith_name_the_title`

Names the program to the host: what its window is titled, and the id a
desktop groups its windows by, a sound server lists it under, and its saves
are kept under among the user's data.

The boot program calls this before the runtime boots, with the names the
lift settled on. Both are kept rather than copied, so they have to outlive
the program, as the string constants the boot program passes do. A program
that never calls it, or passes nothing for one, is named xenolith.

`xenolith_title_name` and `xenolith_title_id` give the two back, for the parts
of the runtime that open a window, a drawing device or a sound stream.

### `xenolith_import_variable`

Gives a variable the kernel exports a home in guest memory, and puts its
address in the slot the title reads it through.

A title imports a few of the kernel's variables as well as its functions, and
reads each through a slot that the console's loader fills with the variable's
address. Nothing else fills it here, so the boot program calls this once for
each slot, after saying what kernel build the title was made against. Each
starts as nothing, and the ones a title reads a value out of are given one: the
graphics clock, the kernel's version, and the clock readings, which are kept
current for as long as the run lasts. One this does not know is reported.

## Shaders

### `xenolith_said`

Finds a shader translated when the title was lifted, by the hash of its words, and returns its SPIR-V. Written by the lift into `shaders.c`.

### `xenolith_reads`

Where a shader goes for its vertices, as five words a stream: which fetch
constant says where it is, how far apart two vertices sit in it, how far into
one this reads, how much of it, and one where the number it fetches by is one
the shader worked out rather than the vertex's own. Nothing where the shader
fetches none.

A host needs these where it has to lay a vertex out itself rather than hand
over what a title wrote, which is what a shape the hardware completes asks
for. Found the same way a shader is, because it belongs to one.

### `xenolith_looks_at`

Which of the places a picture is kept a shader reads one from. Nothing where
it reads none.

A host makes ready only the pictures a shader will look at. Making every place
ready instead means turning a picture round and handing it over for each one a
title has named, whether anything reads it or not, which for a title that
names a whole screen is gigabytes of work a frame that nothing looks at.

### `xenolith_say_a_shader`

Says a shader in a host's terms while a title runs, for one it loaded from
the data beside its executable rather than out of the executable itself.

Returns how many words the answer takes, which is nought where the shader
could not be said. Where that is more than there was room for, nothing is
written and the caller may ask again with more.

Only there where the library that says shaders was linked in.

### `xenolith_can_draw_with`

Whether a pair of shaders can be drawn with, building what a device needs
the first time the pair is asked for.

## Drawing

What the GPU command reader calls on the renderer for each draw.

### `xenolith_can_draw`

Whether there is anywhere to draw, which there is not on a machine with no
graphics driver and in a build made without one. Says so once.

### `xenolith_begin_a_frame`

Starts a frame, cleared to nothing. Reports whether there was anywhere to
start one.

### `xenolith_end_a_frame`

Finishes a frame and reads it back.

### `xenolith_drawing_into`

Says what shape of place a title is about to draw into, so that what each of
them holds is kept apart from what the others hold, and how many samples
across and down each of its pixels is.

### `xenolith_hand_over_constants`

Hands a shader the constants a title wrote, taken from where a title wrote
them and turned into the order a host reads.

### `xenolith_move_a_position_by`

Says what the hardware would have done to a position a shader put out, as
something to multiply by and something to add. Passing nothing says to leave
a position as the shader gave it.

### `xenolith_look_at`

Says where on what is drawn into a position between one and minus one lands,
as the console says it: what to multiply a position by and what to add.
Nothing said means the whole of what is drawn into.

### `xenolith_clip_to`

Keeps drawing inside the window a title named, in the pixels of what is
being drawn into. A window reaching past the edge is brought back to it.

### `xenolith_find_the_pixel_in`

Says which register the next draw's pixel shader finds where its pixel is in,
which a title asks the hardware for rather than working out, or all ones where
it asks for none.

### `xenolith_push_away_by`

Says how much further away what is drawn next is taken to be when it is
compared with what is already there: a distance added outright, as a fraction
of the whole depth, and one added for each sixteenth of a pixel a shape slopes
away by, which is how the console's polygon offset is said. Nought for both
draws as the shape lies.

A title draws a light or a mark flat onto a surface it has already drawn, and
pushes it a little nearer so that it is not half hidden by the surface it lies
on.

### `xenolith_draw_now`

Draws what a title asked to draw, with the shaders it last handed over.

A place is thrown away before it is drawn where how solid it is does not
answer `cut_off`, the console's own three bits, against `cut_off_at`, which
carries the bits of the number they are measured against.

### `xenolith_test_a_draw_is_given`

The test a draw is given, from how a title says colours are laid down and the
mode it draws in. A draw that keeps only how far away things are runs no pixel
shader on the console, so it is given the test that draws everything, whatever
the colour register says.

## Vertices

How a draw's vertex data reaches the translated vertex shader.

### `xenolith_as_a_fetch_reads`

A word of a title's vertices as a fetch reads it, from the word as the processor
reads it and the byte order the fetch constant names: not turned, turned within
each half, turned whole (the processor's own order), or the halves turned about
each other.

### `xenolith_room_for_vertices`

Somewhere to put the vertices a shader goes and gets, and how many words of it
there is room for. Returns nothing where there is nothing to draw with.

### `xenolith_where_the_room_begins`

Where the room last given begins, in words from the front of what a shader
fetches from, and through `given_out` a number that changes whenever room given
earlier is given out again, which says whether words put there earlier are still
there for a later draw to read.

### `xenolith_streams_begin_at`

Says where each of the next draw's streams begins, a word for each of the
ninety six a fetch can name, in words from the front of what a shader fetches
from. Said after the draw's constants are handed over.

### `xenolith_done_with_vertices`

Says the vertices are in place, and how many words of the room given were
written. A frame keeps every draw's words at once, so what one draw took has
to be known before the next is given room.

### `xenolith_number_the_vertices`

Says what number each vertex of the next draw arrives at its shader with: the
`count` words `at` words into the room the vertices were put in, one for each
vertex drawn, or where `count` is nought, one after another from `first`. A
word of all ones in the first starts a new run of shapes.

A shader works out what it fetches from the number it is given, and on the
console that number is the title's own, however the shape is put together.
Laying the vertices out again in the order they are drawn instead, as a host
otherwise has to for a quad, hands the shader numbers the title never gave:
one that reads a particle's corner and centre at a quarter of its number reads
another particle's. So the shape is drawn by the title's numbers.

## Textures

### `xenolith_look_at_a_picture`

Hands over a picture a shader reads, laid out the way a host reads one.

The console keeps a picture in an order that puts the parts near one another
on the screen near one another in memory, and squeezed into squares of four
by four. A host squeezes them the same way, so what is squeezed is handed over
as it is and only the order is undone. Says whether it took.

How it is held says how it is sampled in its lowest twelve bits, and above
them which part of the picture each of the four a shader reads is, three bits
each in the console's numbering.

### `xenolith_look_at_nothing`

Leaves a place a shader reads a picture from holding nothing.

A title fills the same few places over and over, so a place left as it was is
a place still holding a picture the title named for an earlier draw. Where
what it names now cannot be read, that is another of the title's own pictures
drawn in its stead, which looks like a drawing gone wrong rather than like
something not being read.

### `xenolith_forget_pictures_at`

Forgets any picture already turned that came out of memory the title has
since written over, so that the next draw reads what is there now.

## Copies and frames

How drawing gets back into guest memory and onto the screen.

### `xenolith_hand_back_what_was_drawn`

Hands what was drawn back into a title's own memory, as a `struct
xenolith_copy` says. Returns zero where there is nothing to hand back.

The copy is recorded with what was drawn, and the memory is written once that
is handed over: when a frame is shown, when a picture is made out of that
memory, or when `xenolith_wait_for_what_was_drawn` is asked.

The copy names a rectangle in the pixels of the place drawn into, which is
where it lands in the picture copied into as well: a title drawing a frame in
parts moves where the picture starts for each part rather than the rectangle.
How many samples the place keeps across and down for each pixel says how much
of the surface one pixel covers, and those are averaged on the way out.

A title asks for how far away things came out as often as it asks for their
colours, and reads that back as a picture to work out haze, fog and what is
out of focus. Handed the colours instead it draws none of those, so which of
the two is wanted is said in the copy. How far away is kept either as a
fraction of the whole or as the console's small float.

### `xenolith_what_a_copy_reaches`

How many bytes from where a copy starts it writes over, given the pitch it
writes and the right and bottom of what it takes. A copy goes out in squares of
thirty two parts on a side, four bytes each, a row of squares at a time, so it
reaches as far as the last square it writes rather than a whole picture's worth
past where it starts.

### `xenolith_wipe_how_far_after_a_copy`

Says whether the copy just made asked for how far away things are to start
again before the next drawing, and what to start it as.

### `xenolith_wipe_the_colours_after_a_copy`

Says whether the copy just made asked for the colours to start again, and what
to start them as. They are wiped at once, while the surface still holds the
place the copy was made from.

### `xenolith_send_what_was_drawn`

Hands over everything recorded so far without waiting for it to run, so that
the device draws it while the next is recorded. What copies among it read
reaches memory once a later wait, or a picture made out of that memory, asks
for it.

### `xenolith_wait_for_what_was_drawn`

Hands over everything recorded so far, waits for it to run, and writes what
the copies among it read back into the memory their titles named. A copy's
memory holds what was copied once this has been asked for.

### `xenolith_show_what_was_copied_to`

Shows the drawing that was last copied out to where a title names, rather
than whatever has been drawn since. Says whether there was one to show.

### `xenolith_show_the_frame`

Shows whatever is on the drawing surface now, for a title that hands a frame
straight to the display without copying it out first. The copied-frame path
above is preferred where a title uses it; this is the fallback.

### `xenolith_keep_a_frame`

Writes what was drawn where somebody can look at it.

## Input

### `xenolith_what_is_held`

What is held down on the keyboard, as a controller's buttons, the two
thumbsticks across and up, the left one first, and how far the two triggers are
pulled. Returns zero where there is no window to read one from.

### `xenolith_pad_seen`

What a controller was last read as: the buttons, the two sticks, the two
triggers, and the number its last reading carried.

### `xenolith_pad_reading`

The number a controller's reading carries, moved on each time what it holds
changes from what was last seen. A title handed the same number twice takes it
that nothing changed.

## Sound

### `xenolith_frames_kept`

How far a decoder of compressed sound is behind the frames it was handed, kept
for each sound between one packet and the next.

### `xenolith_samples_owed`

Counts the frames a packet finished against the samples the decoder gave for
them, and says how many samples of silence make up for frames the decoder threw
away. The console's decoder gives back every frame, and the streams of one
sound have to stay in step for its channels to line up.
