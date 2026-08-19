//! Turning decoded Xbox 360 functions into C.
//!
//! Everything before this crate reads: the loader decodes an image, the
//! disassembler decodes instructions, and analysis finds functions and the
//! control flow between their blocks. This is where that becomes something that
//! can be compiled.
//!
//! Two rules shape it. An instruction with no semantics is admitted rather than
//! approximated, and a function holding one is not emitted at all. Code that is
//! right except in one place compiles and runs and is wrong, and nothing
//! downstream can tell the difference, so a function is lifted whole or it is
//! reported.
//!
//! The crate contains no `unsafe` code.

mod effect;

pub use effect::{Effect, Location, effect_of, is_modelled, unmodelled};
