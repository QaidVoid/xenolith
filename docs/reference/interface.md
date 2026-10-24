# The runtime interface

Every function `xenolith.h` declares, with the header's own words about what
it is for. The emitted C is written against this and nothing else.

`scripts/check-docs` fails the gate when a declaration here is not named on a
page, so this list and the header cannot drift apart.

## `xenolith_load8`

Declared by the header; see the group it sits in.

## `xenolith_load16`

Declared by the header; see the group it sits in.

## `xenolith_load32`

Declared by the header; see the group it sits in.

## `xenolith_load64`

Declared by the header; see the group it sits in.

## `xenolith_store8`

Declared by the header; see the group it sits in.

## `xenolith_store16`

Declared by the header; see the group it sits in.

## `xenolith_store32`

Declared by the header; see the group it sits in.

## `xenolith_store64`

Declared by the header; see the group it sits in.

## `xenolith_to_a_whole_number`

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

## `xenolith_to_a_whole_word`

The same for the form that gives a 32-bit whole number.

The result is sign extended into the 64-bit register. The instruction does not
define what the upper word holds; sign extension is this implementation's
choice, taken so that the overflow path and the ordinary path agree. The value
out of range is the 32-bit one the console gives, not the 64-bit one: a 32-bit
conversion that overflows has nowhere to put a 64-bit answer.

## `xenolith_to_a_whole_number_by_cutting`

The form that cuts the fraction off rather than rounding.

A cast does exactly this, so the cast is the whole of it. What a cast does not
do is answer for a value outside the range, and that is the same as above: a
value that is not a number, and a value too large, each have the console's one
answer and this states it rather than leaving it to the host.

## `xenolith_to_a_whole_word_by_cutting`

Declared by the header; see the group it sits in.

## `xenolith_vector_u8`

Declared by the header; see the group it sits in.

## `xenolith_vector_set_u8`

Declared by the header; see the group it sits in.

## `xenolith_vector_u16`

Declared by the header; see the group it sits in.

## `xenolith_vector_set_u16`

Declared by the header; see the group it sits in.

## `xenolith_half_to_float`

Turns one of the console's half sized floats into a whole one.

It is not quite the one everybody else keeps: there is no value that stands
for infinity and none that stands for not a number, so the largest exponent
is just the largest exponent, and a value too small to be written with an
exponent of its own is nought rather than being written the long way. Both
follow from the shift being done as it stands, which is why this is written
out rather than handed to a host instruction that would round it the other
way.

## `xenolith_vector_u32`

Declared by the header; see the group it sits in.

## `xenolith_vector_set_u32`

Declared by the header; see the group it sits in.

## `xenolith_vector_u64`

Declared by the header; see the group it sits in.

## `xenolith_vector_set_u64`

Declared by the header; see the group it sits in.

## `xenolith_vector_f32`

A float lane moves through its bits rather than through a pointer aimed at
them, because copying is what the language defines and the pointer is what a
compiler is allowed to assume nothing does.

## `xenolith_vector_set_f32`

Declared by the header; see the group it sits in.

## `xenolith_pack_colour`

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

## `xenolith_pack_short`

One lane of a pair or a quartet of signed halfwords, read the same way.

The window is the thirty two thousand seven hundred and sixty seven either
side of three, and what falls outside it stops at the end it left by. The
low bound is one above the most negative halfword, which is what the console
saturates to rather than the halfword below it.

## `xenolith_pack_half`

One lane of a pair or a quartet of half precision values.

This one is a conversion rather than a reading of bits, so it rounds, and it
rounds to nearest with ties going to even the way every other narrowing in
this instruction set does. The format has no infinity and no value that is
not a number, so a value too large to hold takes the largest exponent and one
that is not a number keeps a mantissa bit set; neither means what the names
would mean in an ordinary half, and widening either gives a finite float.

## `xenolith_saturate_signed`

Converting a single precision value to a fixed point one, with saturation.

The instruction set clamps rather than wrapping, and a value that is not a
number becomes zero. Writing a cast instead would be undefined for exactly
the inputs this has to get right.

## `xenolith_saturate_unsigned`

Declared by the header; see the group it sits in.

## `xenolith_multiply_high`

The high half of a doubleword product.

C has no type wider than the operands to hold it in, so it is built from the
word products the way it would be done by hand. A compiler that has a
double-width type of its own recognizes this and emits the single
instruction; one that does not still gets the right answer.

## `xenolith_multiply_high_signed`

Declared by the header; see the group it sits in.

## `xenolith_vector_shift_left`

Shifting a whole vector register, across lane boundaries.

The amount is in bits and may be anything up to the width of the register, so
a byte of the result generally takes bits from two bytes of the source. The
shifts that move whole bytes are the same operation with a multiple of eight.

## `xenolith_vector_shift_right`

Declared by the header; see the group it sits in.

## `xenolith_clamp`

Clamping a wider intermediate back into a lane.

The saturating forms of the vector arithmetic stop at the end of the range
rather than wrapping, so each is computed at a width that cannot overflow and
then brought back. A clamp at the wrong bound gives an answer that is almost
right, which is the kind that survives review.

## `xenolith_clamp_unsigned`

Declared by the header; see the group it sits in.

## `xenolith_zero_block`

Zeroing a data cache block.

The block is the console's cache line, which is 128 bytes. This console
spells it two ways, and the decoder folds both into one opcode, so both
clear a line. The address is aligned down to the line.

A general purpose emulator does not share the line size, so this is the one
instruction here with no execution oracle behind it.

## `xenolith_condition_pack`

The condition register as one word, and back.

The first field occupies the most significant four bits, and within a field
the order is less than, greater than, equal, summary overflow. Both
directions are written here rather than at each use, because the same bit
order mistake made in both would cancel out and survive.

The mask names fields from the first, so its most significant bit selects
field zero.

## `xenolith_condition_unpack`

Declared by the header; see the group it sits in.

## `xenolith_trap`

Where a trap that fired goes.

A trap leaves the function it was in, so what happens next is not something
emitted code can express. The environment decides. Implemented there, not
here.

## `xenolith_break`

A title asking for a debugger, with the number saying why. A trap that always
fires, against register nought and a number, is how a title spells that, and a
console with no debugger attached carries on past it. So does this, after saying
where it was asked for the first time each place asks.

## `xenolith_dispatch`

Where a target unknown when the code was emitted becomes a function.

This is the single place an address is resolved at run time, so that a branch
whose table could not be read has one path rather than many. Implemented by
the environment, not here.

## `xenolith_unlifted`

Where a call to a function that was never translated goes.

Told apart from a trap because the two are different events. A trap is an
instruction the title ran on purpose, and this is a function the translation
could not express, so reading one as the other sends a reader looking in the
wrong place. Implemented by the environment, not here.

## `xenolith_import`

Where a call to an imported function goes.

The container names an import by ordinal within a library and never by name,
so the ordinal is what the title actually said. The name beside it comes from
a catalogue of what the console's libraries export and is null where the
catalogue has no entry, so a reader gets something to act on without the
ordinal ever being replaced by a guess at it.

What the ordinal does is the environment's to decide. Implemented there, not
here.

## `xenolith_reserve32`

Where a reservation is taken and where it is redeemed.

A program with one thread and no sharing could load and store these directly
and be right, which is exactly why they are not emitted that way: the
assumption would sit where nothing could find it. An environment that has
threads implements these with real atomics instead.

The conditional store reports whether the store happened, which is what the
retry branch after one reads.

## `xenolith_reserve64`

Declared by the header; see the group it sits in.

## `xenolith_conditional32`

Declared by the header; see the group it sits in.

## `xenolith_conditional64`

Declared by the header; see the group it sits in.

## `xenolith_timebase`

The time base, which advances.

At what rate is the environment's decision. Emitting a constant would produce
a program whose timing loops never finish.

## `xenolith_lookup`

Where an address becomes the function lifted for it.

Only the lift knows which addresses it emitted, so the table is written
beside the code rather than here. Anything not in it is not a function this
program has.

## `xenolith_boot`

Mapping guest memory and loading the image into it.

Emitted code reaches memory by adding a guest address to a base, so the base
has to be a mapping wide enough that any address it could form indexes
inside it. Returns the base, or a null pointer if the image could not be
read.

## `xenolith_processor_block`

The block a title reaches through r13, which is what it reads the kernel's
tick count and its own processor number out of.

A title that finds nothing there reads processor nought on every thread, and
its vector code picks a slot from that number and steps a counter in it, so
two threads on one block step the same counter and carry it past the end.

## `xenolith_say_a_shader`

Says a shader in a host's terms while a title runs, for one it loaded from
the data beside its executable rather than out of the executable itself.

Returns how many words the answer takes, which is nought where the shader
could not be said. Where that is more than there was room for, nothing is
written and the caller may ask again with more.

Only there where the library that says shaders was linked in.

## `xenolith_said`

Declared by the header; see the group it sits in.

## `xenolith_reads`

Where a shader goes for its vertices, as five words a stream: which fetch
constant says where it is, how far apart two vertices sit in it, how far into
one this reads, how much of it, and one where the number it fetches by is one
the shader worked out rather than the vertex's own. Nothing where the shader
fetches none.

A host needs these where it has to lay a vertex out itself rather than hand
over what a title wrote, which is what a shape the hardware completes asks
for. Found the same way a shader is, because it belongs to one.

## `xenolith_looks_at`

Which of the places a picture is kept a shader reads one from. Nothing where
it reads none.

A host makes ready only the pictures a shader will look at. Making every place
ready instead means turning a picture round and handing it over for each one a
title has named, whether anything reads it or not, which for a title that
names a whole screen is gigabytes of work a frame that nothing looks at.

## `xenolith_can_draw`

Whether there is anywhere to draw, which there is not on a machine with no
graphics driver and in a build made without one. Says so once.

## `xenolith_hand_over_constants`

Hands a shader the constants a title wrote, taken from where a title wrote
them and turned into the order a host reads.

## `xenolith_move_a_position_by`

Says what the hardware would have done to a position a shader put out, as
something to multiply by and something to add. Passing nothing says to leave
a position as the shader gave it.

## `xenolith_clip_to`

Keeps drawing inside the window a title named, in the pixels of what is
being drawn into. A window reaching past the edge is brought back to it.

## `xenolith_look_at_a_picture`

Hands over a picture a shader reads, laid out the way a host reads one.

The console keeps a picture in an order that puts the parts near one another
on the screen near one another in memory, and squeezed into squares of four
by four. A host squeezes them the same way, so what is squeezed is handed over
as it is and only the order is undone. Says whether it took.

How it is held says how it is sampled in its lowest twelve bits, and above
them which part of the picture each of the four a shader reads is, three bits
each in the console's numbering.

## `xenolith_look_at_nothing`

Leaves a place a shader reads a picture from holding nothing.

A title fills the same few places over and over, so a place left as it was is
a place still holding a picture the title named for an earlier draw. Where
what it names now cannot be read, that is another of the title's own pictures
drawn in its stead, which looks like a drawing gone wrong rather than like
something not being read.

## `xenolith_send_what_was_drawn`

Hands over everything recorded so far without waiting for it to run, so that
the device draws it while the next is recorded. What copies among it read
reaches memory once a later wait, or a picture made out of that memory, asks
for it.

## `xenolith_what_a_copy_reaches`

How many bytes from where a copy starts it writes over, given the pitch it
writes and the right and bottom of what it takes. A copy goes out in squares of
thirty two parts on a side, four bytes each, a row of squares at a time, so it
reaches as far as the last square it writes rather than a whole picture's worth
past where it starts.

## `xenolith_wait_for_what_was_drawn`

Hands over everything recorded so far, waits for it to run, and writes what
the copies among it read back into the memory their titles named. A copy's
memory holds what was copied once this has been asked for.

## `xenolith_drawing_into`

Says what shape of place a title is about to draw into, so that what each of
them holds is kept apart from what the others hold, and how many samples
across and down each of its pixels is.

## `xenolith_find_the_pixel_in`

Says which register the next draw's pixel shader finds where its pixel is in,
which a title asks the hardware for rather than working out, or all ones where
it asks for none.

## `xenolith_wipe_how_far_after_a_copy`

Says whether the copy just made asked for how far away things are to start
again before the next drawing, and what to start it as.

## `xenolith_wipe_the_colours_after_a_copy`

Says whether the copy just made asked for the colours to start again, and what
to start them as. They are wiped at once, while the surface still holds the
place the copy was made from.

## `xenolith_built_against`

Says which kit a title was built against, read out of its container.

The graphics library a title links keeps its device in a layout that changes
between kits, and the runtime answers a few words of that layout: how far the
hardware has got, in the two forms the library keeps. Told the kit, it knows
where those words are; told nothing, it answers neither and a title that
watches them decides the hardware has stopped.

## `xenolith_privileges`

Says what a title's own container declares it is allowed to do.

A title asks whether it holds a numbered privilege, and the answer is a bit
in the system flags word its executable carries. That word is in the
container and nowhere else, so it is passed in rather than guessed at. A
title told it holds nothing takes the path for a build that was not licensed
to do what it does, which on the titles tried so far is to shut itself down
before it draws anything.

Nought means the container did not say, which is answered the same way as a
word with the bit clear: nothing here invents a privilege.

## `xenolith_import_variable`

Gives a variable the kernel exports a home in guest memory, and puts its
address in the slot the title reads it through.

A title imports a few of the kernel's variables as well as its functions, and
reads each through a slot that the console's loader fills with the variable's
address. Nothing else fills it here, so the boot program calls this once for
each slot, after saying what kernel build the title was made against. Each
starts as nothing, and the ones a title reads a value out of are given one: the
graphics clock, the kernel's version, and the clock readings, which are kept
current for as long as the run lasts. One this does not know is reported.

## `xenolith_room_for_vertices`

Somewhere to put the vertices a shader goes and gets, and how many words of it
there is room for. Returns nothing where there is nothing to draw with.

## `xenolith_done_with_vertices`

Says the vertices are in place, and how many words of the room given were
written. A frame keeps every draw's words at once, so what one draw took has
to be known before the next is given room.

## `xenolith_number_the_vertices`

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

## `xenolith_forget_pictures_at`

Forgets any picture already turned that came out of memory the title has
since written over, so that the next draw reads what is there now.

## `xenolith_show_what_was_copied_to`

Shows the drawing that was last copied out to where a title names, rather
than whatever has been drawn since. Says whether there was one to show.

## `xenolith_show_the_frame`

Shows whatever is on the drawing surface now, for a title that hands a frame
straight to the display without copying it out first. The copied-frame path
above is preferred where a title uses it; this is the fallback.

## `xenolith_look_at`

Says where on what is drawn into a position between one and minus one lands,
as the console says it: what to multiply a position by and what to add.
Nothing said means the whole of what is drawn into.

## `xenolith_hand_back_what_was_drawn`

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

## `xenolith_what_is_held`

What is held down on the keyboard, as a controller's buttons and the two
thumbsticks. Returns zero where there is no window to read one from.

## `xenolith_begin_a_frame`

Starts a frame, cleared to nothing. Reports whether there was anywhere to
start one.

## `xenolith_draw_now`

Draws what a title asked to draw, with the shaders it last handed over.

A place is thrown away before it is drawn where how solid it is does not
answer `cut_off`, the console's own three bits, against `cut_off_at`, which
carries the bits of the number they are measured against.

## `xenolith_end_a_frame`

Finishes a frame and reads it back.

## `xenolith_can_draw_with`

Whether a pair of shaders can be drawn with, building what a device needs
the first time the pair is asked for.

## `xenolith_keep_a_frame`

Writes what was drawn where somebody can look at it.
