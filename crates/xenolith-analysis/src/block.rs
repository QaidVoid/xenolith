//! Basic blocks, and walking code to find them.
//!
//! A basic block is a run of instructions entered only at its first and left
//! only at its last. Finding them is the first thing that turns a stream of
//! decoded words into something with structure.
//!
//! Walking follows control flow rather than reading straight through, because
//! reading straight through cannot tell where one function stops and the data
//! between functions begins. Every reason a walk stops is recorded rather than
//! collapsed into a single failure, since the difference between reaching a
//! return, running into something that does not decode, and running out of
//! section is exactly what a later stage needs to judge whether a function was
//! understood.

use std::collections::{BTreeMap, BTreeSet};

use xenolith_ppc::{FlowKind, Instruction};
use xenolith_xex::Image;

/// Bytes in one instruction.
pub(crate) const INSTRUCTION_SIZE: u32 = 4;

/// Why a block stopped where it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminator {
    /// The last instruction transfers control.
    Transfer {
        /// What kind of transfer it is.
        kind: FlowKind,
        /// Where it goes, when the instruction alone determines that.
        target: Option<u32>,
        /// Whether the following instruction is reachable from it.
        falls_through: bool,
    },
    /// The next word does not decode, so the walk stopped rather than guessing
    /// whether what follows is code.
    Undecodable {
        /// Address of the word that did not decode.
        at: u32,
    },
    /// The block reached the end of the executable range it sits in.
    SectionEnd,
    /// The following instruction begins another block, so this one ends without
    /// transferring control.
    FallsInto {
        /// Address the next block starts at.
        next: u32,
    },
}

impl Terminator {
    /// Returns whether the instruction after this block is reachable from it.
    #[must_use]
    pub const fn falls_through(&self) -> bool {
        match self {
            Self::Transfer { falls_through, .. } => *falls_through,
            Self::FallsInto { .. } => true,
            Self::Undecodable { .. } | Self::SectionEnd => false,
        }
    }

    /// Returns the address this block transfers to, when it determines one.
    #[must_use]
    pub const fn target(&self) -> Option<u32> {
        match self {
            Self::Transfer { target, .. } => *target,
            _ => None,
        }
    }
}

/// A run of instructions entered only at its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    /// Address of the first instruction.
    pub start: u32,
    /// Address one past the last instruction.
    pub end: u32,
    /// Why the block stopped.
    pub terminator: Terminator,
}

impl Block {
    /// Returns how many instructions the block holds.
    #[must_use]
    pub const fn len(&self) -> u32 {
        (self.end - self.start) / INSTRUCTION_SIZE
    }

    /// Returns whether the block holds no instructions.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.end == self.start
    }

    /// Returns whether an address falls inside the block.
    #[must_use]
    pub const fn contains(&self, address: u32) -> bool {
        address >= self.start && address < self.end
    }
}

/// Returns whether an address is in an executable section of the image.
fn is_executable(image: &Image, address: u32) -> bool {
    image
        .section_at(address)
        .is_some_and(|section| section.kind.is_executable())
}

/// Walks control flow from `entry`, returning the blocks it reaches.
///
/// Follows the taken and fall through paths of every transfer it decodes, so
/// what comes back is the code reachable from `entry` rather than everything
/// between two addresses. Blocks do not overlap and together cover every
/// instruction the walk decoded.
#[must_use]
pub fn blocks_from(image: &Image, entry: u32) -> Vec<Block> {
    let mut leaders = BTreeSet::new();
    let mut ends = BTreeMap::new();
    let mut pending = vec![entry];

    leaders.insert(entry);

    while let Some(start) = pending.pop() {
        if ends.contains_key(&start) || !is_executable(image, start) {
            continue;
        }

        let mut address = start;
        loop {
            // Running into a leader means the rest of this run is another
            // block, which has been or will be walked on its own.
            if address != start && leaders.contains(&address) {
                ends.insert(start, (address, Terminator::FallsInto { next: address }));
                break;
            }

            if !is_executable(image, address) {
                ends.insert(start, (address, Terminator::SectionEnd));
                break;
            }

            let Ok(word) = image.u32(address) else {
                ends.insert(start, (address, Terminator::SectionEnd));
                break;
            };

            let instruction = Instruction::decode(word);
            if instruction.is_unknown() {
                ends.insert(start, (address, Terminator::Undecodable { at: address }));
                break;
            }

            let next = address.saturating_add(INSTRUCTION_SIZE);
            let flow = instruction.flow(address);

            if flow.terminates_block() {
                if let Some(target) = flow.target {
                    if is_executable(image, target) {
                        leaders.insert(target);
                        pending.push(target);
                    }
                }
                if flow.falls_through && is_executable(image, next) {
                    leaders.insert(next);
                    pending.push(next);
                }

                let terminator = Terminator::Transfer {
                    kind: flow.kind,
                    target: flow.target,
                    falls_through: flow.falls_through,
                };
                ends.insert(start, (next, terminator));
                break;
            }

            if next <= address {
                // The address space wrapped, so there is nowhere further to go.
                ends.insert(start, (next, Terminator::SectionEnd));
                break;
            }
            address = next;
        }
    }

    split(&leaders, &ends)
}

/// Turns walked runs into blocks that stop at every leader.
///
/// A run walked before a leader inside it was discovered has to be cut, or two
/// blocks would overlap and the instruction at the leader would belong to both.
fn split(leaders: &BTreeSet<u32>, ends: &BTreeMap<u32, (u32, Terminator)>) -> Vec<Block> {
    let mut blocks = Vec::with_capacity(ends.len());

    for (&start, &(end, terminator)) in ends {
        let cut = leaders.range((start + 1)..end).next().copied();

        match cut {
            Some(next) => blocks.push(Block {
                start,
                end: next,
                terminator: Terminator::FallsInto { next },
            }),
            None => blocks.push(Block {
                start,
                end,
                terminator,
            }),
        }
    }

    blocks.sort_by_key(|block| block.start);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ImageBuilder, encode};

    #[test]
    fn a_straight_run_is_one_block() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::addi(3, 4, 1), encode::addi(3, 3, 1), encode::blr()])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start, 0x8200_0000);
        assert_eq!(blocks[0].end, 0x8200_000c);
        assert_eq!(blocks[0].len(), 3);
        assert!(matches!(
            blocks[0].terminator,
            Terminator::Transfer {
                kind: FlowKind::Return,
                ..
            }
        ));
    }

    #[test]
    fn a_forward_branch_splits_the_run_it_lands_in() {
        // A conditional branch skipping one instruction, so its target sits
        // inside the run that follows it.
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[
                encode::bc(12, 0, 8),
                encode::addi(3, 4, 1),
                encode::addi(3, 3, 1),
                encode::blr(),
            ])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);
        let starts: Vec<u32> = blocks.iter().map(|b| b.start).collect();

        assert_eq!(starts, vec![0x8200_0000, 0x8200_0004, 0x8200_0008]);
        assert_eq!(blocks[0].len(), 1, "the branch ends its block");
        assert_eq!(blocks[1].len(), 1, "the skipped instruction stands alone");
    }

    #[test]
    fn a_backward_branch_makes_a_loop_without_walking_forever() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[
                encode::addi(3, 3, 1),
                encode::bc(12, 0, encode::back(4)),
                encode::blr(),
            ])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].terminator.target(), Some(0x8200_0000));
    }

    #[test]
    fn blocks_do_not_overlap_and_cover_every_instruction() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[
                encode::bc(12, 0, 12),
                encode::addi(3, 4, 1),
                encode::b(8),
                encode::addi(3, 5, 1),
                encode::blr(),
            ])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        for pair in blocks.windows(2) {
            assert!(pair[0].end <= pair[1].start, "blocks overlap: {pair:?}");
        }
        let covered: u32 = blocks.iter().map(Block::len).sum();
        assert_eq!(covered, 5, "every instruction belongs to a block");
    }

    #[test]
    fn a_word_that_does_not_decode_ends_the_block() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::addi(3, 4, 1), 0x17ff_ffff, encode::blr()])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].len(), 1);
        assert_eq!(
            blocks[0].terminator,
            Terminator::Undecodable { at: 0x8200_0004 }
        );
    }

    #[test]
    fn a_block_stops_at_the_end_of_the_section() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::addi(3, 4, 1), encode::addi(3, 3, 1)])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].len(), 2);
        assert_eq!(blocks[0].terminator, Terminator::SectionEnd);
    }

    #[test]
    fn a_call_falls_through_to_the_following_instruction() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::bl(8), encode::blr(), encode::blr()])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert!(matches!(
            blocks[0].terminator,
            Terminator::Transfer {
                kind: FlowKind::Call,
                falls_through: true,
                ..
            }
        ));
        assert!(
            blocks.iter().any(|b| b.start == 0x8200_0004),
            "the instruction after a call is reachable"
        );
    }

    #[test]
    fn an_unconditional_branch_does_not_reach_what_follows_it() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::b(8), encode::addi(3, 4, 1), encode::blr()])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert!(
            !blocks.iter().any(|b| b.contains(0x8200_0004)),
            "the skipped instruction is not reached"
        );
        assert!(blocks.iter().any(|b| b.start == 0x8200_0008));
    }

    #[test]
    fn walking_from_outside_an_executable_section_finds_nothing() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::blr()])
            .build();

        assert!(blocks_from(&image, 0x9000_0000).is_empty());
    }

    #[test]
    fn an_indirect_branch_terminates_without_a_target() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::bctr()])
            .build();

        let blocks = blocks_from(&image, 0x8200_0000);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].terminator.target(), None);
        assert!(matches!(
            blocks[0].terminator,
            Terminator::Transfer {
                kind: FlowKind::Indirect,
                ..
            }
        ));
    }
}
