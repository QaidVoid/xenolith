//! Disassembler for the shader instruction set of the Xbox 360 graphics
//! processor.
//!
//! A shader arrives as words rather than as a file. A title hands one to the
//! hardware inside the commands it writes, saying which of two kinds it is and
//! how many words follow, so what this reads is a run of words whose length is
//! known and whose meaning is not.
//!
//! Two shapes share that run. A control flow instruction is forty eight bits,
//! so they come two to every three words, and each says which run of the others
//! to execute. The others are ninety six bits, one to every three words, and
//! each does arithmetic or fetches something. Which part of the run holds which
//! is not marked: the control flow comes first and says where the rest begins.
//!
//! Decoding never fails. A word encoding nothing this crate recognizes reports
//! an unknown operation rather than being guessed at, because a wrong decode
//! becomes silently wrong output while an unknown one is a loud failure at the
//! time of reading.
pub use alu::{Alu, ScalarOperation, VectorOperation};
pub use fetch::{Fetch, FetchKind, Format, Packed};
pub use find::{Found, reads_as_shader, scan};
pub use flow::{Flow, FlowOperation};
pub use translate::{Refused, Stream, Translated, translate};

mod alu;
mod fetch;
mod find;
mod flow;
pub mod opcodes;
#[allow(unsafe_code)]
pub mod running;
pub mod spirv;
mod translate;

/// How many words one control flow instruction takes, counted in halves because
/// two of them share three words.
pub const FLOW_HALVES_PER_WORDS: (usize, usize) = (2, 3);

/// How many words one arithmetic instruction takes.
pub const ALU_WORDS: usize = 3;

/// Which of the two kinds of shader a run of words holds.
///
/// A title says which when it hands the run over, rather than the run saying so
/// itself, so this is what it said rather than anything read back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Runs once for each vertex, and where a draw counts its vertices rather
    /// than fetching them, this is what turns a count into a position.
    Vertex,
    /// Runs once for each pixel covered.
    Pixel,
}

impl Kind {
    /// What a title's own number for the kind means, or nothing where it is a
    /// number this does not know.
    #[must_use]
    pub fn from_word(word: u32) -> Option<Self> {
        match word {
            0 => Some(Self::Vertex),
            1 => Some(Self::Pixel),
            _ => None,
        }
    }
}

/// Reads a word from a run, or nothing where the run is too short.
///
/// Every read goes through here so that a run whose length a title got wrong
/// stops the reading rather than taking whatever sits after it.
#[must_use]
fn word_at(words: &[u32], at: usize) -> Option<u32> {
    words.get(at).copied()
}

/// What a run of words comes to, for finding a translation again by the words
/// it was made from.
///
/// A title hands the hardware the same shader over and over, and what arrives
/// is words rather than a name, so the only thing to match on is the words. The
/// sum below is the one a runtime works out the same way, so the two agree
/// without either knowing what the other did.
#[must_use]
pub fn sum_of(words: &[u32]) -> u32 {
    let mut sum: u32 = 0x811c_9dc5;
    for word in words {
        for byte in word.to_be_bytes() {
            sum ^= u32::from(byte);
            sum = sum.wrapping_mul(0x0100_0193);
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::sum_of;

    #[test]
    fn the_same_words_come_to_the_same_thing() {
        assert_eq!(sum_of(&[1, 2, 3]), sum_of(&[1, 2, 3]));
    }

    #[test]
    fn different_words_come_to_different_things() {
        assert_ne!(sum_of(&[1, 2, 3]), sum_of(&[1, 2, 4]));
        assert_ne!(sum_of(&[1, 2]), sum_of(&[1, 2, 0]));
    }
}
