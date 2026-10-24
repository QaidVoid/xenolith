//! The numbers the hardware's own layout is made of.
//!
//! How many registers a shader has, how many constants it is handed, how many
//! things it carries on to what runs after it, and where among the constants the
//! ones a title fills in while it runs sit. None of these is a choice made here:
//! each is the console's own number, and where a smaller one was tried the
//! reason it failed is written beside it.

/// How many pictures a shader can be handed, which is how many the hardware
/// keeps a place for.
pub(crate) const TEXTURES: u32 = 32;

/// The decoration that says a constant is settled when a pair of shaders is
/// built rather than when the module is written.
pub(crate) const SAID_WHEN_BUILT: u32 = 1;

/// The numbers the settled constants are asked for by.
pub(crate) const WHICH_TEST: u32 = 0;
pub(crate) const WHAT_IT_IS_AGAINST: u32 = 1;
pub(crate) const WHERE_THE_PIXEL_GOES: u32 = 2;

/// What the register a pixel's position goes into is settled as where a title
/// asks for it in none.
pub(crate) const NO_REGISTER: u32 = u32::MAX;

/// The test that draws a place however solid it is, which is what a title that
/// asks for no test at all comes to. Its three bits stand for drawing a place
/// less solid than the test, exactly as solid, and more solid, so all three
/// together draw everything and a driver folds the whole test away.
pub(crate) const DRAWN_WHATEVER_IT_IS: u32 = 7;

/// Which export a vertex shader puts the position of the vertex in.
///
/// The console's own number for it, and what this title's shaders use: they
/// export to nought and one, which are carried on to whatever runs next, and to
/// this, which is where the vertex went.
pub const POSITION: u8 = 62;

/// How many things a shader carries on to whatever runs after it.
///
/// Sixteen, which is how many the console has. Fewer was tried, on the grounds
/// that the shaders looked at named only the first four, and a title turned out
/// to name up to the seventh: a shader naming one past the end is refused
/// whole, so whatever it drew is not drawn at all. The number the hardware has
/// is the only one that cannot be wrong.
pub const CARRIED: u32 = 16;

/// How many more than the constants a shader names are handed to it.
///
/// Twelve, which is what the two things kept beside the constants need.
///
/// The first two are what the hardware would have done to a position after the
/// shader put it out, as something to multiply by and something to add. On the
/// console that is a set of registers rather than a constant, and whether it is
/// applied at all is a register too, so a host cannot be told once and for all.
/// Handing it over beside the constants is how the same shader serves both a
/// title that leaves the hardware to do it and one that has already done it
/// itself.
///
/// The other eight are the truths and the counts a shader asks after when it
/// jumps, at [`THE_TRUTHS`] and [`THE_COUNTS`]. Twelve is the exact number those
/// need, not a round one: two for the scale and the offset, two more before the
/// truths, and eight for the counts.
pub(crate) const BESIDE_THE_CONSTANTS: u32 = 12;

/// Where among the constants the truths a shader asks after when it jumps are
/// kept: after the scale and the offset, two lots of four words holding the two
/// hundred and fifty six of them a bit apiece.
pub(crate) const THE_TRUTHS: u32 = ALL_THE_CONSTANTS + 2;

/// Where among the constants the counts sit: eight lots of four words holding
/// the thirty two a shader reads to know how many times to go round a loop.
pub(crate) const THE_COUNTS: u32 = ALL_THE_CONSTANTS + 4;

/// How many constants the hardware holds altogether.
///
/// Twice what one shader names, because the two kinds of shader do not share
/// them: what a vertex shader calls its first constant is the file's first, and
/// what a pixel shader calls its first is the file's two hundred and fifty
/// seventh. The file runs to where the constants a fetch reads begin, which is
/// what says there are this many and not some other number.
pub(crate) const ALL_THE_CONSTANTS: u32 = 512;

/// How many constants a shader is handed.
///
/// The console's own number for the float constants a shader can name, and more
/// than the highest one this title's shaders reach.
pub(crate) const CONSTANTS: u32 = 256;

/// How many registers a shader is given.
///
/// The console's own number, and what the six bits naming a destination can
/// reach.
pub(crate) const REGISTERS: u32 = 64;
