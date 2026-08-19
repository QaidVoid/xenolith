//! Function and control flow recovery for Xbox 360 executables.
//!
//! Reads an image through the address space the loader exposes and decodes it
//! through the disassembler, adding no understanding of the container format or
//! the instruction encoding of its own.
//!
//! What this produces is structure: where the functions are, how control moves
//! between their blocks, and which transfers could not be resolved. It reasons
//! about the shape of code and never about what a computation produces, so
//! nothing here interprets a value.
//!
//! Where something cannot be established, it is reported as unrecovered rather
//! than guessed at. A missing edge that looks present is worse than one that
//! admits it is missing, because every later stage inherits the lie.
//!
//! The crate contains no `unsafe` code.

mod block;
mod function;
mod helper;
#[cfg(test)]
mod testing;

pub use block::{Block, Terminator, blocks_from, blocks_within};
pub use function::{Edge, Function, Origin, Program, analyze};
pub use helper::{Helper, HelperDirection, HelperKind, Helpers, detect};
