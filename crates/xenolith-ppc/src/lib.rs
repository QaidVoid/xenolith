//! Disassembler for the PowerPC instruction set of the Xbox 360 CPU.
//!
//! Decodes a 32-bit instruction word into a typed value carrying its operation,
//! with operands read on demand from the word it kept. Decoding never fails: a
//! word encoding nothing this crate recognizes reports [`Opcode::Unknown`]
//! rather than being guessed at, because a wrong decode becomes silently wrong
//! generated code while an unknown one is a loud failure at analysis time.
//!
//! The target is the 64-bit PowerPC base instruction set plus `AltiVec` and the
//! console's VMX128 extension. Instruction sets added to the architecture after
//! the Xbox 360 shipped are deliberately not decoded, since several of them
//! claim encoding space that VMX128 uses.
//!
//! This is an original implementation. The instruction table is written from
//! published architecture documentation and the public record for VMX128, and
//! derives from no existing decoder.
//!
//! The crate contains no `unsafe` code and allocates nothing while decoding.

#[cfg(test)]
mod differential;
mod form;
mod instruction;
mod table;

pub use form::Form;
pub use instruction::Instruction;
pub use table::Opcode;
