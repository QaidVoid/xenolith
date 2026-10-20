//! Finding shaders in an image.
//!
//! A title hands a shader to the hardware at the moment it wants it, out of its
//! own data, so nothing in the image says here is a shader. What says so is the
//! shape: a run of words reads as one, or it does not.
//!
//! The test is that the parts agree. Every stretch of arithmetic a shader says
//! to run has to begin where the last one finished, the stretches have to cover
//! what follows them with nothing left over and nothing twice, and the whole
//! thing has to stop on the one operation that means stop. Words that are not a
//! shader fail that on the first stretch that points somewhere the last one did
//! not reach.
//!
//! What this is not is a claim that everything it finds is used. A title
//! decides while running which of them it wants, and something that finds one
//! it never asks for has cost a translation and nothing else.

use crate::{ALU_WORDS, Flow, FlowOperation};

/// The fewest words a shader can be: one group of three holding the operation
/// that means stop, and one instruction for it to run.
const SHORTEST: usize = 6;

/// Where a shader begins in a run of words, and how many words it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    /// Which word it starts at.
    pub at: usize,
    /// How many words it takes, counting the padding it ends with.
    pub words: usize,
}

/// How long the run of words starting here reads as a shader, or nothing where
/// it does not read as one.
///
/// The length returned is where the control flow stops and the arithmetic it
/// covers ends, which is what a title would hand over.
#[must_use]
pub fn reads_as_shader(words: &[u32]) -> Option<usize> {
    /* A shader begins with something to do rather than with padding.
     *
     * Padding reads as a shader just as well as the shader after it does, so
     * without this a run would be reported starting from every word of nothing
     * in front of it, and where it starts is what a title hands over. Every one
     * a title was seen to hand over begins with a stretch to run or with room
     * being reserved, never with nothing. */
    let (first, _) = Flow::pair(words, 0)?;
    if first.operation() == FlowOperation::Nothing {
        return None;
    }

    let mut next: Option<u16> = None;
    let mut ended = false;
    let mut furthest: u16 = 0;
    let mut begins: Option<u16> = None;
    let mut at = 0;

    while let Some((low, high)) = Flow::pair(words, at) {
        for flow in [low, high] {
            if ended {
                /* Past the end only padding is allowed, because a shader that
                 * says to run something after it has stopped is words that are
                 * not a shader. */
                if !matches!(
                    flow.operation(),
                    FlowOperation::Nothing | FlowOperation::Reserve
                ) {
                    return None;
                }
                continue;
            }

            /* Anything else is a reason to say no, an operation with no reading
             * as much as one this could follow but has not been shown to need.
             * Words that are not a shader reach one of those quickly. */
            match flow.operation() {
                FlowOperation::Run | FlowOperation::RunAndEnd => {
                    let count = u16::from(flow.count());
                    if count == 0 {
                        return None;
                    }
                    if let Some(expected) = next {
                        if flow.address() != expected {
                            return None;
                        }
                    }
                    if begins.is_none() {
                        begins = Some(flow.address());
                    }
                    let finish = flow.address().checked_add(count)?;
                    next = Some(finish);
                    furthest = furthest.max(finish);
                    if flow.ends_the_shader() {
                        ended = true;
                    }
                }
                FlowOperation::Nothing | FlowOperation::Reserve => {}
                _ => return None,
            }
        }

        at += ALU_WORDS;

        if ended {
            /* The arithmetic begins where the control flow stops.
             *
             * Both are counted in the same threes, so the first stretch a
             * shader runs names the group the control flow ends on. Without
             * tying the two together a single stretch pointing a long way off
             * reads as a shader thousands of words long, which is what a run of
             * anything at all will eventually do. */
            let begins = begins?;
            if usize::from(begins).checked_mul(ALU_WORDS)? != at {
                return None;
            }

            let covered = usize::from(furthest).checked_mul(ALU_WORDS)?;
            if covered < at {
                return None;
            }
            return if covered <= words.len() {
                Some(covered)
            } else {
                None
            };
        }
    }

    None
}

/// Every place in a run of words that reads as a shader.
///
/// Where one is found, the search carries on after it rather than inside it, so
/// a shader is not reported again from each of its own instructions.
#[must_use]
pub fn scan(words: &[u32]) -> Vec<Found> {
    let mut all = Vec::new();
    let mut at = 0;

    while at + SHORTEST <= words.len() {
        let rest = words.get(at..).unwrap_or_default();
        if let Some(length) = reads_as_shader(rest) {
            all.push(Found { at, words: length });
            at += length;
        } else {
            at += 1;
        }
    }

    all
}

#[cfg(test)]
mod tests {
    use super::{reads_as_shader, scan};

    /// The pixel shader from the real shader tests, which is the shortest one a
    /// title was seen to hand over.
    const PIXEL: [u32; 9] = [
        0x0000_0000,
        0x1001_c400,
        0x2200_0000,
        0xc80f_8000,
        0x0000_0000,
        0xc200_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
    ];

    #[test]
    fn a_shader_reads_as_one() {
        assert_eq!(reads_as_shader(&PIXEL), Some(6));
    }

    #[test]
    fn nothing_reads_as_a_shader() {
        assert!(reads_as_shader(&[0; 12]).is_none());
    }

    #[test]
    fn counting_up_does_not_read_as_a_shader() {
        let words: Vec<u32> = (0..32u32).collect();
        assert!(reads_as_shader(&words).is_none());
    }

    #[test]
    fn a_shader_is_found_once_rather_than_from_each_of_its_words() {
        let mut words = vec![0u32; 3];
        words.extend_from_slice(&PIXEL);
        words.extend_from_slice(&[0; 3]);
        let found = scan(&words);
        assert_eq!(found.len(), 1);
        assert_eq!(found.first().map(|one| one.at), Some(3));
    }
}
