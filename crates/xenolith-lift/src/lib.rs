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
mod emit;

/// The interface emitted code is written against, shipped alongside it.
pub const RUNTIME_HEADER: &str = include_str!("../runtime/xenolith.h");

/// An implementation of that interface, enough to link and no more.
///
/// It maps guest memory and reports what it declines to do. No import is
/// serviced and no thread exists, so a program built against it stops at the
/// first call into the operating system.
pub const RUNTIME_SOURCE: &str = include_str!("../runtime/xenolith.c");

pub use effect::{Effect, Location, effect_of, is_modelled, unmodelled};
pub use emit::{
    Imported, Imports, Lifted, Unlifted, code_for, declaration_of, is_liftable, lift, name_of,
};
