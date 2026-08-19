//! Finding functions and what reaches them.
//!
//! Discovery starts from the addresses known to be code without being derived,
//! which is the entry point and anything the caller supplies, and follows every
//! direct call outward until nothing new turns up.
//!
//! Function boundaries and the walk depend on each other. Where a function ends
//! is decided by where the next one begins, and the next one is only found by
//! walking the one before it. Discovery therefore runs to a fixed point: walk
//! with the boundaries known so far, take the call targets that turns up, and
//! walk again if any were new. Doing it in one pass would let a tail call pull
//! the function it jumps to into its caller, and the two would be reported as
//! one.

use std::collections::{BTreeMap, BTreeSet};

use xenolith_ppc::FlowKind;
use xenolith_xex::Image;

use crate::block::{Block, Terminator, blocks_within};
use crate::helper::{Helpers, detect};

/// How a function came to be known about.
///
/// A function reached by a call is known to be one. A function found any other
/// way is believed to be one, and later stages may want to weigh the two
/// differently, so the distinction is kept rather than flattened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Origin {
    /// The image names it as where execution begins.
    EntryPoint,
    /// The caller supplied it, from an export table or otherwise.
    Root,
    /// Something calls it directly.
    Called,
}

impl Origin {
    /// Returns a readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EntryPoint => "entry point",
            Self::Root => "root",
            Self::Called => "called",
        }
    }
}

/// Where control goes when it leaves a block.
///
/// Edges describe movement inside one function. A transfer that leaves it, by
/// returning, by calling, or by tail calling into another function, produces no
/// edge here, because the block it would point at is not this function's to
/// reason about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    /// Control transfers to another block of the same function.
    Taken(u32),
    /// Control continues at the instruction after the block.
    FallThrough(u32),
    /// Control leaves and nothing here determines where.
    ///
    /// Carried rather than dropped. A function missing an edge looks fully
    /// analyzed while part of it was never reached, and every later stage would
    /// inherit that.
    Unresolved,
}

impl Edge {
    /// Returns the block this edge points at, when it points at one.
    #[must_use]
    pub const fn target(self) -> Option<u32> {
        match self {
            Self::Taken(target) | Self::FallThrough(target) => Some(target),
            Self::Unresolved => None,
        }
    }
}

/// A discovered function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    /// Address of the first instruction.
    pub start: u32,
    /// How it came to be known about.
    pub origin: Origin,
    /// Its blocks, in address order.
    pub blocks: Vec<Block>,
    /// Addresses it leaves for without expecting to come back, where the target
    /// is another function.
    pub tail_calls: Vec<u32>,
}

impl Function {
    /// Returns the address one past the highest instruction the function holds.
    #[must_use]
    pub fn end(&self) -> u32 {
        self.blocks
            .iter()
            .map(|block| block.end)
            .max()
            .unwrap_or(self.start)
    }

    /// Returns how many instructions the function holds.
    #[must_use]
    pub fn instruction_count(&self) -> u32 {
        self.blocks.iter().map(Block::len).sum()
    }

    /// Returns whether an address falls inside one of the function's blocks.
    ///
    /// Blocks are checked rather than the range between the first and the last,
    /// because a function whose code is not contiguous would otherwise claim
    /// the gaps.
    #[must_use]
    pub fn contains(&self, address: u32) -> bool {
        self.blocks.iter().any(|block| block.contains(address))
    }

    /// Returns the addresses this function calls directly.
    pub fn calls(&self) -> impl Iterator<Item = u32> + '_ {
        self.blocks
            .iter()
            .filter_map(|block| match block.terminator {
                Terminator::Transfer {
                    kind: FlowKind::Call,
                    target: Some(target),
                    ..
                } => Some(target),
                _ => None,
            })
    }

    /// Returns the outgoing edges of every block, keyed by the block's start.
    ///
    /// Computed rather than stored, so a function assembled by hand behaves the
    /// same as one that came out of discovery.
    #[must_use]
    pub fn edges(&self) -> BTreeMap<u32, Vec<Edge>> {
        let starts: BTreeSet<u32> = self.blocks.iter().map(|block| block.start).collect();

        self.blocks
            .iter()
            .map(|block| (block.start, edges_of(block, &starts)))
            .collect()
    }

    /// Returns the blocks reachable from the function's first block.
    #[must_use]
    pub fn reachable_blocks(&self) -> BTreeSet<u32> {
        let edges = self.edges();
        let mut reachable = BTreeSet::new();
        let mut pending = vec![self.start];

        while let Some(start) = pending.pop() {
            if !reachable.insert(start) {
                continue;
            }
            for edge in edges.get(&start).into_iter().flatten() {
                if let Some(target) = edge.target() {
                    pending.push(target);
                }
            }
        }

        reachable
    }

    /// Returns the blocks no edge reaches.
    ///
    /// Reported rather than removed. A block nothing reaches is evidence that
    /// something was misread, and deleting it would hide that.
    #[must_use]
    pub fn unreachable_blocks(&self) -> Vec<&Block> {
        let reachable = self.reachable_blocks();
        self.blocks
            .iter()
            .filter(|block| !reachable.contains(&block.start))
            .collect()
    }

    /// Returns how many outgoing edges leave without a known target.
    #[must_use]
    pub fn unresolved_edge_count(&self) -> usize {
        self.edges()
            .values()
            .flatten()
            .filter(|edge| **edge == Edge::Unresolved)
            .count()
    }

    /// Returns whether any block leaves without the target being known.
    #[must_use]
    pub fn has_unresolved_transfer(&self) -> bool {
        self.blocks.iter().any(|block| {
            matches!(
                block.terminator,
                Terminator::Transfer {
                    kind: FlowKind::Indirect,
                    target: None,
                    ..
                }
            )
        })
    }
}

/// What analysis found in an image.
#[derive(Debug, Clone, Default)]
pub struct Program {
    helpers: Helpers,
    functions: BTreeMap<u32, Function>,
}

impl Program {
    /// Returns the detected save and restore helpers.
    #[must_use]
    pub const fn helpers(&self) -> &Helpers {
        &self.helpers
    }

    /// Returns every discovered function, in address order.
    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.functions.values()
    }

    /// Returns how many functions were discovered.
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// Returns how many functions came to be known a particular way.
    #[must_use]
    pub fn count_from(&self, origin: Origin) -> usize {
        self.functions
            .values()
            .filter(|function| function.origin == origin)
            .count()
    }

    /// Returns the function starting at an address, if one does.
    #[must_use]
    pub fn function_at(&self, address: u32) -> Option<&Function> {
        self.functions.get(&address)
    }

    /// Returns the functions whose blocks hold an address.
    ///
    /// More than one may, since functions are allowed to share code.
    pub fn functions_containing(&self, address: u32) -> impl Iterator<Item = &Function> {
        self.functions
            .values()
            .filter(move |function| function.contains(address))
    }

    /// Returns how many instructions all discovered functions hold together,
    /// counting an instruction once however many functions share it.
    #[must_use]
    pub fn claimed_instructions(&self) -> u64 {
        let mut claimed = BTreeSet::new();
        for function in self.functions.values() {
            for block in &function.blocks {
                let mut address = block.start;
                while address < block.end {
                    claimed.insert(address);
                    address = address.saturating_add(4);
                }
            }
        }
        claimed.len() as u64
    }
}

/// Returns the outgoing edges of one block.
fn edges_of(block: &Block, starts: &BTreeSet<u32>) -> Vec<Edge> {
    let mut edges = Vec::new();

    match block.terminator {
        Terminator::Transfer {
            kind,
            target,
            falls_through,
        } => {
            match kind {
                // Returning and calling both leave for somewhere this function
                // does not describe. A call comes back, and that is the fall
                // through below rather than an edge to the callee.
                FlowKind::Return | FlowKind::Call | FlowKind::Continue => {}
                // A branch through a register has no intra function successor
                // on its taken side until a table is recovered for it.
                FlowKind::Indirect => edges.push(Edge::Unresolved),
                FlowKind::Branch => match target {
                    Some(target) if starts.contains(&target) => {
                        edges.push(Edge::Taken(target));
                    }
                    // A branch to a known address outside this function is a
                    // tail call, recorded separately.
                    Some(_) => {}
                    None => edges.push(Edge::Unresolved),
                },
            }

            if falls_through && starts.contains(&block.end) {
                edges.push(Edge::FallThrough(block.end));
            }
        }
        Terminator::FallsInto { next } => {
            if starts.contains(&next) {
                edges.push(Edge::FallThrough(next));
            }
        }
        // Neither reaches anything further.
        Terminator::Undecodable { .. } | Terminator::SectionEnd => {}
    }

    edges
}

/// Returns whether an address is in an executable section.
fn is_executable(image: &Image, address: u32) -> bool {
    image
        .section_at(address)
        .is_some_and(|section| section.kind.is_executable())
}

/// Discovers the functions of an image, along with its helpers.
///
/// Seeds from the image entry point and from `roots`, which is where an export
/// table's addresses belong once the loader can enumerate them.
#[must_use]
pub fn analyze(image: &Image, roots: &[u32]) -> Program {
    let helpers = detect(image);

    let mut origins: BTreeMap<u32, Origin> = BTreeMap::new();
    if let Some(entry) = image.entry_point() {
        if is_executable(image, entry) {
            origins.insert(entry, Origin::EntryPoint);
        }
    }
    for &root in roots {
        if is_executable(image, root) {
            origins.entry(root).or_insert(Origin::Root);
        }
    }

    // Boundaries and walks define each other, so this runs until neither
    // changes. Each round walks with what is known and learns from what it saw.
    let walked: BTreeMap<u32, Vec<Block>> = loop {
        let boundaries: BTreeSet<u32> = origins.keys().copied().collect();

        let walked: BTreeMap<u32, Vec<Block>> = boundaries
            .iter()
            .map(|&start| (start, blocks_within(image, start, &boundaries)))
            .collect();

        let mut discovered = Vec::new();
        for blocks in walked.values() {
            for block in blocks {
                let Terminator::Transfer {
                    kind: FlowKind::Call,
                    target: Some(target),
                    ..
                } = block.terminator
                else {
                    continue;
                };

                // A call into a helper is a call to that helper, not to a
                // function beginning wherever the caller happened to enter.
                if !is_executable(image, target)
                    || helpers.containing(target).is_some()
                    || origins.contains_key(&target)
                {
                    continue;
                }
                discovered.push(target);
            }
        }

        if discovered.is_empty() {
            break walked;
        }
        for target in discovered {
            origins.entry(target).or_insert(Origin::Called);
        }
    };

    let starts: BTreeSet<u32> = origins.keys().copied().collect();
    let functions = walked
        .into_iter()
        .map(|(start, blocks)| {
            let origin = origins.get(&start).copied().unwrap_or(Origin::Called);
            let tail_calls = tail_calls_of(&blocks, start, &starts);
            (
                start,
                Function {
                    start,
                    origin,
                    blocks,
                    tail_calls,
                },
            )
        })
        .collect();

    Program { helpers, functions }
}

/// Returns the branches leaving a function for the start of another one.
///
/// A tail call is compiled as a plain branch, so it is only distinguishable
/// from an ordinary one by where it lands.
fn tail_calls_of(blocks: &[Block], start: u32, starts: &BTreeSet<u32>) -> Vec<u32> {
    let mut targets: Vec<u32> = blocks
        .iter()
        .filter_map(|block| match block.terminator {
            Terminator::Transfer {
                kind: FlowKind::Branch,
                target: Some(target),
                ..
            } if target != start && starts.contains(&target) => Some(target),
            _ => None,
        })
        .collect();

    targets.sort_unstable();
    targets.dedup();
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ImageBuilder, encode};

    /// Builds an image whose entry point is its first instruction.
    fn image(words: &[u32]) -> Image {
        ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(words)
            .build()
    }

    #[test]
    fn the_entry_point_is_a_function() {
        let program = analyze(&image(&[encode::blr()]), &[]);

        assert_eq!(program.function_count(), 1);
        let function = program.function_at(0x8200_0000).unwrap();
        assert_eq!(function.origin, Origin::EntryPoint);
    }

    #[test]
    fn a_called_address_becomes_a_function() {
        // The entry calls forward past its own return.
        let program = analyze(&image(&[encode::bl(8), encode::blr(), encode::blr()]), &[]);

        assert_eq!(program.function_count(), 2);
        assert_eq!(
            program.function_at(0x8200_0008).unwrap().origin,
            Origin::Called
        );
    }

    #[test]
    fn a_call_chain_is_followed_to_its_end() {
        let program = analyze(
            &image(&[
                encode::bl(12),
                encode::blr(),
                encode::blr(),
                encode::bl(8),
                encode::blr(),
                encode::blr(),
            ]),
            &[],
        );

        assert_eq!(program.function_count(), 3);
        assert!(program.function_at(0x8200_000c).is_some());
        assert!(program.function_at(0x8200_0014).is_some());
    }

    #[test]
    fn a_supplied_root_is_discovered_without_being_called() {
        let words = [encode::blr(), encode::addi(3, 4, 1), encode::blr()];
        let program = analyze(&image(&words), &[0x8200_0004]);

        assert_eq!(program.function_count(), 2);
        assert_eq!(
            program.function_at(0x8200_0004).unwrap().origin,
            Origin::Root
        );
    }

    #[test]
    fn direct_recursion_terminates() {
        let program = analyze(&image(&[encode::bl(encode::back(0)), encode::blr()]), &[]);

        assert_eq!(program.function_count(), 1);
    }

    #[test]
    fn mutual_recursion_terminates() {
        let program = analyze(
            &image(&[
                encode::bl(8),
                encode::blr(),
                encode::bl(encode::back(8)),
                encode::blr(),
            ]),
            &[],
        );

        assert_eq!(program.function_count(), 2);
    }

    #[test]
    fn a_call_to_an_unmapped_address_does_not_stop_the_walk() {
        let program = analyze(
            &image(&[
                encode::bl(0x0010_0000),
                encode::addi(3, 4, 1),
                encode::blr(),
            ]),
            &[],
        );

        assert_eq!(program.function_count(), 1, "no function is invented");
        assert_eq!(
            program
                .function_at(0x8200_0000)
                .unwrap()
                .instruction_count(),
            3,
            "the walk carried on past the call"
        );
    }

    /// A tail call is a plain branch, so the only thing marking it is that it
    /// lands where another function begins.
    #[test]
    fn a_branch_into_another_function_is_a_tail_call() {
        // The first function calls the third, then branches into it.
        let program = analyze(
            &image(&[
                encode::bl(8),
                encode::b(4),
                encode::addi(3, 4, 1),
                encode::blr(),
            ]),
            &[],
        );

        let first = program.function_at(0x8200_0000).unwrap();
        assert_eq!(first.tail_calls, vec![0x8200_0008]);
    }

    /// Without a boundary the branch would pull the other function's blocks in
    /// and the two would be reported as one.
    #[test]
    fn a_tail_call_does_not_absorb_the_function_it_reaches() {
        let program = analyze(
            &image(&[
                encode::bl(8),
                encode::b(4),
                encode::addi(3, 4, 1),
                encode::blr(),
            ]),
            &[],
        );

        let first = program.function_at(0x8200_0000).unwrap();
        assert!(
            !first.contains(0x8200_0008),
            "the tail called function stayed its own"
        );
        assert!(program.function_at(0x8200_0008).is_some());
    }

    #[test]
    fn a_call_into_a_helper_does_not_become_a_function() {
        let mut words = vec![encode::bl(8), encode::blr()];
        for i in 0..18u32 {
            let displacement = encode::back(152).wrapping_add(i.wrapping_mul(8));
            words.push(encode::std(14 + i, 1, displacement & 0xffff));
        }
        words.push(encode::blr());
        let program = analyze(&image(&words), &[]);

        assert!(
            program.function_at(0x8200_0008).is_none(),
            "the helper is not a function"
        );
        assert!(program.helpers().containing(0x8200_0008).is_some());
    }

    #[test]
    fn functions_may_share_a_block() {
        // Two functions whose bodies converge on the same return.
        let program = analyze(
            &image(&[encode::bl(8), encode::b(8), encode::b(4), encode::blr()]),
            &[],
        );

        let sharing: Vec<u32> = program
            .functions_containing(0x8200_000c)
            .map(|function| function.start)
            .collect();

        assert!(!sharing.is_empty(), "the shared block belongs to someone");
    }

    #[test]
    fn an_indirect_transfer_is_reported_as_unresolved() {
        let program = analyze(&image(&[encode::bctr()]), &[]);

        assert!(
            program
                .function_at(0x8200_0000)
                .unwrap()
                .has_unresolved_transfer()
        );
    }

    /// A conditional branch may or may not be taken, so both paths are edges.
    #[test]
    fn a_conditional_branch_has_a_taken_and_a_fall_through_edge() {
        let program = analyze(
            &image(&[encode::bc(12, 0, 8), encode::addi(3, 4, 1), encode::blr()]),
            &[],
        );

        let function = program.function_at(0x8200_0000).unwrap();
        let edges = function.edges();
        let first = &edges[&0x8200_0000];

        assert!(first.contains(&Edge::Taken(0x8200_0008)));
        assert!(first.contains(&Edge::FallThrough(0x8200_0004)));
        assert_eq!(first.len(), 2);
    }

    #[test]
    fn an_unconditional_branch_has_only_a_taken_edge() {
        let program = analyze(
            &image(&[encode::b(8), encode::addi(3, 4, 1), encode::blr()]),
            &[],
        );

        let function = program.function_at(0x8200_0000).unwrap();
        let edges = function.edges();

        assert_eq!(edges[&0x8200_0000], vec![Edge::Taken(0x8200_0008)]);
    }

    #[test]
    fn a_return_has_no_successor_inside_the_function() {
        let program = analyze(&image(&[encode::blr()]), &[]);

        let function = program.function_at(0x8200_0000).unwrap();

        assert!(function.edges()[&0x8200_0000].is_empty());
    }

    /// A call leaves for another function and comes back, so its only edge
    /// inside this one is to the instruction it comes back to.
    #[test]
    fn a_call_falls_through_and_does_not_edge_to_its_callee() {
        let program = analyze(&image(&[encode::bl(8), encode::blr(), encode::blr()]), &[]);

        let function = program.function_at(0x8200_0000).unwrap();
        let edges = function.edges();

        assert_eq!(edges[&0x8200_0000], vec![Edge::FallThrough(0x8200_0004)]);
    }

    #[test]
    fn an_indirect_branch_carries_an_unresolved_edge() {
        let program = analyze(&image(&[encode::bctr()]), &[]);

        let function = program.function_at(0x8200_0000).unwrap();

        assert_eq!(function.edges()[&0x8200_0000], vec![Edge::Unresolved]);
        assert_eq!(function.unresolved_edge_count(), 1);
    }

    /// An indirect call still returns to the instruction after it, so its
    /// successor inside the function is known even though its callee is not.
    #[test]
    fn an_indirect_call_is_not_an_unresolved_edge() {
        // bctrl: branch to the count register, taking the link.
        let program = analyze(&image(&[0x4e80_0421, encode::blr()]), &[]);

        let function = program.function_at(0x8200_0000).unwrap();

        assert_eq!(function.unresolved_edge_count(), 0);
        assert_eq!(
            function.edges()[&0x8200_0000],
            vec![Edge::FallThrough(0x8200_0004)]
        );
    }

    #[test]
    fn a_tail_call_produces_no_edge_inside_the_function() {
        let program = analyze(
            &image(&[
                encode::bl(8),
                encode::b(4),
                encode::addi(3, 4, 1),
                encode::blr(),
            ]),
            &[],
        );

        let function = program.function_at(0x8200_0000).unwrap();
        let edges = function.edges();

        assert!(
            edges[&0x8200_0004].is_empty(),
            "the branch leaves the function, so it is a tail call not an edge"
        );
        assert_eq!(function.tail_calls, vec![0x8200_0008]);
    }

    #[test]
    fn every_block_of_a_discovered_function_is_reachable() {
        let program = analyze(
            &image(&[
                encode::bc(12, 0, 12),
                encode::addi(3, 4, 1),
                encode::b(8),
                encode::addi(3, 5, 1),
                encode::blr(),
            ]),
            &[],
        );

        let function = program.function_at(0x8200_0000).unwrap();

        assert!(
            function.unreachable_blocks().is_empty(),
            "discovery only walks what it can reach"
        );
        assert_eq!(function.reachable_blocks().len(), function.blocks.len());
    }

    /// Discovery cannot produce one of these, since it only walks what it
    /// reaches, so the reporting is checked against a function assembled by
    /// hand. It matters because a block nothing reaches is evidence that
    /// something was misread, and removing it would hide that.
    #[test]
    fn a_block_nothing_reaches_is_reported_rather_than_removed() {
        let orphan = Block {
            start: 0x8200_0100,
            end: 0x8200_0104,
            terminator: Terminator::SectionEnd,
        };
        let function = Function {
            start: 0x8200_0000,
            origin: Origin::Root,
            blocks: vec![
                Block {
                    start: 0x8200_0000,
                    end: 0x8200_0004,
                    terminator: Terminator::Transfer {
                        kind: FlowKind::Return,
                        target: None,
                        falls_through: false,
                    },
                },
                orphan,
            ],
            tail_calls: Vec::new(),
        };

        let unreachable = function.unreachable_blocks();

        assert_eq!(unreachable.len(), 1);
        assert_eq!(unreachable[0].start, 0x8200_0100);
        assert_eq!(function.blocks.len(), 2, "it is still in the function");
    }

    #[test]
    fn every_edge_points_at_a_block_of_the_same_function() {
        let program = analyze(
            &image(&[
                encode::bc(12, 0, 12),
                encode::bl(16),
                encode::b(4),
                encode::blr(),
                encode::blr(),
            ]),
            &[],
        );

        for function in program.functions() {
            let starts: BTreeSet<u32> = function.blocks.iter().map(|block| block.start).collect();
            for edges in function.edges().values() {
                for edge in edges {
                    if let Some(target) = edge.target() {
                        assert!(
                            starts.contains(&target),
                            "{target:#010x} is not a block of the function at {:#010x}",
                            function.start
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn counts_are_split_by_how_functions_were_found() {
        let words = [encode::bl(8), encode::blr(), encode::blr()];
        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::EntryPoint), 1);
        assert_eq!(program.count_from(Origin::Called), 1);
        assert_eq!(program.count_from(Origin::Root), 0);
    }

    #[test]
    fn claimed_instructions_counts_a_shared_one_once() {
        let program = analyze(
            &image(&[encode::bl(8), encode::b(8), encode::b(4), encode::blr()]),
            &[],
        );

        assert!(program.claimed_instructions() <= 4);
    }

    #[test]
    fn an_image_with_no_entry_point_discovers_nothing() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::blr()])
            .build();

        assert_eq!(analyze(&image, &[]).function_count(), 0);
    }

    #[test]
    fn analysis_terminates_over_arbitrary_words() {
        let mut state = 0x5eed_1234u32;
        let words: Vec<u32> = (0..2048)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state
            })
            .collect();

        let _ = analyze(&image(&words), &[]);
    }
}
