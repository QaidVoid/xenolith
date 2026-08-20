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

use xenolith_ppc::{FlowKind, Instruction, Opcode};
use xenolith_xex::Image;

use crate::block::{Block, INSTRUCTION_SIZE, Terminator, blocks_within};
use crate::helper::{HelperDirection, Helpers, detect};

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
    /// Nothing reaches it directly, and it was found by recognizing the shape
    /// of a function prologue.
    ///
    /// A function reached by a call is known to be one. This one is believed to
    /// be one, which is why the two are counted apart.
    Scanned,
}

impl Origin {
    /// Returns a readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EntryPoint => "entry point",
            Self::Root => "root",
            Self::Called => "called",
            Self::Scanned => "scanned",
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
    /// Where each resolved indirect branch inside it can go, keyed by the
    /// address of the branch.
    ///
    /// A branch absent from this map was not resolved, which is a normal
    /// outcome rather than a missing entry.
    pub resolved: BTreeMap<u32, Vec<u32>>,
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
            .map(|block| (block.start, edges_of(block, &starts, &self.resolved)))
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
fn edges_of(
    block: &Block,
    starts: &BTreeSet<u32>,
    resolved: &BTreeMap<u32, Vec<u32>>,
) -> Vec<Edge> {
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
                // on its taken side until a table is recovered for it. Once one
                // is, the targets inside this function are its real successors.
                FlowKind::Indirect => {
                    let branch = block.end.saturating_sub(INSTRUCTION_SIZE);

                    // A dense switch names the same target many times, and a
                    // hundred identical edges say nothing the one edge does not.
                    let mut seen = BTreeSet::new();
                    for target in resolved.get(&branch).into_iter().flatten() {
                        if starts.contains(target) && seen.insert(*target) {
                            edges.push(Edge::Taken(*target));
                        }
                    }

                    // An unresolved branch, and one whose every target lies
                    // outside this function, both leave nothing here to point
                    // at. Saying so is the honest answer for either.
                    if edges.is_empty() {
                        edges.push(Edge::Unresolved);
                    }
                }
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

/// The special purpose register holding where a function was called from.
const LINK_REGISTER: u32 = 8;

/// How many instructions from the start a prologue is looked for within.
const PROLOGUE_WINDOW: u32 = 8;

/// How many independent signals a prologue must show.
///
/// One is not enough. A table of constants will occasionally hold a word that
/// decodes as something a prologue does, and seeding from it would invent a
/// function out of data.
const PROLOGUE_SIGNALS: usize = 2;

/// How many times scanning, recovery, and walking may alternate.
///
/// Each round can only add function starts and entry addresses, and there are
/// finitely many addresses, so the alternation settles on its own. The bound is
/// there so that termination does not depend on that argument holding for every
/// input, which matters more now that a recovered table can name an address
/// that leads back to the branch that named it.
const MAX_SCAN_ROUNDS: usize = 16;

/// Counts the signs of a function prologue at an address.
///
/// The three come from reading real code. A function reads the link register
/// out so it can be saved, calls a shared helper to save the registers it will
/// use, and moves the stack pointer down to make room. Any one alone is weak,
/// and together they are not something data does by accident.
fn prologue_signals(image: &Image, address: u32, helpers: &Helpers) -> usize {
    let mut signals = 0;

    for step in 0..PROLOGUE_WINDOW {
        let at = address.wrapping_add(step * 4);
        let Ok(word) = image.u32(at) else {
            break;
        };
        let instruction = Instruction::decode(word);
        if instruction.is_unknown() {
            break;
        }

        // Reading out where the function was called from, so it survives.
        if instruction.opcode() == Opcode::Mfspr && instruction.spr() == LINK_REGISTER {
            signals += 1;
        }

        // Moving the stack pointer down to make room, in the one instruction
        // that both stores through it and updates it.
        if instruction.opcode() == Opcode::Stwu
            && instruction.rt() == 1
            && instruction.ra() == 1
            && instruction.displacement() < 0
        {
            signals += 1;
        }

        // Calling the shared helper that saves registers.
        let flow = instruction.flow(at);
        let calls_a_save_helper = flow.kind == FlowKind::Call
            && flow.target.is_some_and(|target| {
                helpers
                    .containing(target)
                    .is_some_and(|helper| helper.direction == HelperDirection::Save)
            });
        if calls_a_save_helper {
            signals += 1;
        }
    }

    signals
}

/// Returns addresses that look like functions in ranges nothing claimed.
///
/// Discovery reaches what direct calls reach. Anything called only through a
/// pointer, which in compiled code means everything behind a virtual table, is
/// invisible to it, and this is the pass that goes looking.
fn scan_for_prologues(
    image: &Image,
    helpers: &Helpers,
    walked: &BTreeMap<u32, Vec<Block>>,
) -> Vec<u32> {
    let mut claimed = BTreeSet::new();
    for blocks in walked.values() {
        for block in blocks {
            let mut address = block.start;
            while address < block.end {
                claimed.insert(address);
                address = address.saturating_add(4);
            }
        }
    }

    let mut found = Vec::new();
    for section in image.executable_sections() {
        let mut address = section.start;

        while u64::from(address) < section.end() {
            let next = address.saturating_add(4);
            if next <= address {
                break;
            }

            if claimed.contains(&address) || helpers.containing(address).is_some() {
                address = next;
                continue;
            }

            if prologue_signals(image, address, helpers) >= PROLOGUE_SIGNALS {
                found.push(address);
                // Everything this one reaches is walked before scanning
                // resumes, so there is no point stepping through its body here.
                address = address.saturating_add(PROLOGUE_WINDOW * 4);
                continue;
            }

            address = next;
        }
    }

    found
}

/// Returns whether an address is in an executable section.
fn is_executable(image: &Image, address: u32) -> bool {
    image
        .section_at(address)
        .is_some_and(|section| section.kind.is_executable())
}

/// Returns whether an address could hold an instruction.
///
/// Instructions are four bytes and four byte aligned, so an address that is not
/// cannot be the start of one. Walking from it decodes a stream shifted out of
/// step with the real one, which produces output that looks plausible and is
/// entirely wrong, and claims addresses no instruction ever sits at.
fn is_instruction_address(image: &Image, address: u32) -> bool {
    address % INSTRUCTION_SIZE == 0 && is_executable(image, address)
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
        if is_instruction_address(image, entry) {
            origins.insert(entry, Origin::EntryPoint);
        }
    }
    for &root in roots {
        if is_instruction_address(image, root) {
            origins.entry(root).or_insert(Origin::Root);
        }
    }

    // The other addresses each function is entered at, and where each of its
    // indirect branches goes. Both are filled in by recovery, which needs
    // functions to work from, so both start empty.
    let mut entries: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut resolved: BTreeMap<u32, BTreeMap<u32, Vec<u32>>> = BTreeMap::new();

    let mut walked = walk_to_fixed_point(image, &helpers, &mut origins, &entries);

    // Discovery reaches what calls reach. Scanning looks for what they do not,
    // recovery looks for what a switch reaches, and discovery runs again from
    // whatever either finds.
    //
    // All three alternate rather than running in a fixed order. Walking again
    // with more boundaries can shorten a function that had covered a region,
    // and the code it no longer claims is code scanning skipped over precisely
    // because it had been claimed. Walking into a switch body can expose a
    // further switch. One pass of each leaves all of that unfound.
    for _ in 0..MAX_SCAN_ROUNDS {
        let mut added = false;

        for address in scan_for_prologues(image, &helpers, &walked) {
            if let std::collections::btree_map::Entry::Vacant(slot) = origins.entry(address) {
                slot.insert(Origin::Scanned);
                added = true;
            }
        }

        for function in assemble(&walked, &origins, &resolved).values() {
            for table in crate::jumptable::recover(image, function).recovered() {
                let targets: Vec<u32> =
                    table.targets.iter().copied().chain(table.default).collect();

                let branches = resolved.entry(function.start).or_default();
                if branches.insert(table.branch, targets.clone()).as_deref()
                    != Some(targets.as_slice())
                {
                    added = true;
                }

                let known = entries.entry(function.start).or_default();
                for target in targets {
                    added |= known.insert(target);
                }
            }
        }

        if !added {
            break;
        }
        walked = walk_to_fixed_point(image, &helpers, &mut origins, &entries);
    }

    let functions = assemble(&walked, &origins, &resolved);
    Program { helpers, functions }
}

/// Returns a function for the address a caller enters a helper at.
///
/// A helper is one run of stores or loads that several callers enter at
/// different offsets, depending on how many registers they want saved. A call
/// to one is a call to the helper rather than to a function beginning wherever
/// the caller happened to enter, so discovery does not claim those offsets and
/// the program reports eight helpers rather than a hundred and fifty near
/// duplicates of them.
///
/// Emitted code still needs something to call. This builds the straight line
/// from an entry to the helper's return, so the caller reaches a body rather
/// than a hole.
///
/// Returns nothing for an address no helper covers.
#[must_use]
pub fn helper_entry(image: &Image, helpers: &Helpers, address: u32) -> Option<Function> {
    helpers.containing(address)?;

    let blocks = crate::block::blocks_from(image, address);
    if blocks.is_empty() {
        return None;
    }

    Some(Function {
        start: address,
        origin: Origin::Called,
        blocks,
        tail_calls: Vec::new(),
        resolved: BTreeMap::new(),
    })
}

/// Builds the functions of a walk, attaching what is known about each.
fn assemble(
    walked: &BTreeMap<u32, Vec<Block>>,
    origins: &BTreeMap<u32, Origin>,
    resolved: &BTreeMap<u32, BTreeMap<u32, Vec<u32>>>,
) -> BTreeMap<u32, Function> {
    let starts: BTreeSet<u32> = origins.keys().copied().collect();

    walked
        .iter()
        .map(|(&start, blocks)| {
            let origin = origins.get(&start).copied().unwrap_or(Origin::Called);
            let tail_calls = tail_calls_of(blocks, start, &starts);
            (
                start,
                Function {
                    start,
                    origin,
                    blocks: blocks.clone(),
                    tail_calls,
                    resolved: resolved.get(&start).cloned().unwrap_or_default(),
                },
            )
        })
        .collect()
}

/// Walks every known function until following calls turns up nothing new.
///
/// Boundaries and walks define each other, so this runs until neither changes.
/// Each round walks with what is known and learns from what it saw.
fn walk_to_fixed_point(
    image: &Image,
    helpers: &Helpers,
    origins: &mut BTreeMap<u32, Origin>,
    entries: &BTreeMap<u32, BTreeSet<u32>>,
) -> BTreeMap<u32, Vec<Block>> {
    let none = BTreeSet::new();

    loop {
        let boundaries: BTreeSet<u32> = origins.keys().copied().collect();

        let walked: BTreeMap<u32, Vec<Block>> = boundaries
            .iter()
            .map(|&start| {
                let also = entries.get(&start).unwrap_or(&none);
                (start, blocks_within(image, start, &boundaries, also))
            })
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
    }
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
            resolved: BTreeMap::new(),
        };

        let unreachable = function.unreachable_blocks();

        assert_eq!(unreachable.len(), 1);
        assert_eq!(unreachable[0].start, 0x8200_0100);
        assert_eq!(function.blocks.len(), 2, "it is still in the function");
    }

    /// Builds a function whose single block ends in a branch through a register.
    fn switching(targets: &[u32], resolved: bool) -> Function {
        let branch = 0x8200_0000;
        let mut blocks = vec![Block {
            start: branch,
            end: branch + 4,
            terminator: Terminator::Transfer {
                kind: FlowKind::Indirect,
                target: None,
                falls_through: false,
            },
        }];
        for at in 0..u32::try_from(targets.len()).unwrap_or(0) {
            let start = 0x8200_0100 + at * 4;
            blocks.push(Block {
                start,
                end: start + 4,
                terminator: Terminator::Transfer {
                    kind: FlowKind::Return,
                    target: None,
                    falls_through: false,
                },
            });
        }

        Function {
            start: branch,
            origin: Origin::Root,
            blocks,
            tail_calls: Vec::new(),
            resolved: if resolved {
                BTreeMap::from([(branch, targets.to_vec())])
            } else {
                BTreeMap::new()
            },
        }
    }

    /// Emits a switch on `index` whose table is absolute, placed so that the
    /// branch lands where the caller laid the image out.
    ///
    /// The default is the first word after the branch, and the table is read
    /// from `table`. Eight words long.
    fn switch_on(index: u32, table: u32, default_offset: u32) -> [u32; 8] {
        [
            encode::cmpli(index, 1),
            encode::bc(12, 1, default_offset),
            encode::addis(12, 0, 0x8200),
            encode::addi(12, 12, table & 0xffff),
            encode::slwi(0, index, 2),
            encode::lwzx(0, 12, 0),
            encode::mtctr(0),
            encode::bctr(),
        ]
    }

    /// A switch body is reachable only by reading the table, so a walk that does
    /// not read it leaves the whole body unclaimed.
    #[test]
    fn a_switch_body_is_claimed() {
        let mut words = Vec::new();
        words.extend(switch_on(10, 0x0030, 0x1c));
        words.extend([encode::blr(); 4]);
        words.extend([0x8200_0024, 0x8200_0028]);

        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&words)
            .build();

        let program = analyze(&image, &[]);
        let function = program
            .functions()
            .find(|function| function.start == 0x8200_0000)
            .expect("the entry point is a function");

        for target in [0x8200_0020, 0x8200_0024, 0x8200_0028] {
            assert!(function.contains(target), "{target:#010x} was not claimed");
        }
        assert!(
            function.unreachable_blocks().is_empty(),
            "a block was claimed without the edge that justifies it"
        );
    }

    /// Reading one table can expose a second switch, whose table can then be
    /// read. One round of recovery finds the first and stops.
    #[test]
    fn a_switch_reached_only_through_another_switch_is_claimed() {
        let mut words = Vec::new();
        // The first switch, its default and filler, then its table naming the
        // second switch and one ordinary target.
        words.extend(switch_on(10, 0x0030, 0x1c));
        words.extend([encode::blr(); 4]);
        words.extend([0x8200_0038, 0x8200_0024]);
        // The second switch at 0x82000038, reachable only through that table.
        words.extend(switch_on(11, 0x0068, 0x1c));
        words.extend([encode::blr(); 4]);
        words.extend([0x8200_005c, 0x8200_0060]);

        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&words)
            .build();

        let program = analyze(&image, &[]);
        let function = program
            .functions()
            .find(|function| function.start == 0x8200_0000)
            .expect("the entry point is a function");

        assert!(
            function.contains(0x8200_0038),
            "the second switch was not reached"
        );
        for target in [0x8200_005c, 0x8200_0060] {
            assert!(
                function.contains(target),
                "{target:#010x} is behind the second table and was not claimed"
            );
        }
        assert!(function.unreachable_blocks().is_empty());
    }

    #[test]
    fn a_resolved_branch_reports_an_edge_per_target() {
        let targets = [0x8200_0100, 0x8200_0104, 0x8200_0108];
        let function = switching(&targets, true);

        let edges = function.edges();
        let from_branch = edges.get(&0x8200_0000).expect("the branch has edges");

        assert_eq!(
            from_branch,
            &[
                Edge::Taken(0x8200_0100),
                Edge::Taken(0x8200_0104),
                Edge::Taken(0x8200_0108)
            ]
        );
    }

    /// A dense switch names the same target many times, and repeating the edge
    /// says nothing the one edge does not.
    #[test]
    fn repeated_targets_produce_one_edge_each() {
        let targets = [0x8200_0100, 0x8200_0104, 0x8200_0100, 0x8200_0100];
        let function = switching(&targets, true);

        let edges = function.edges();
        let from_branch = edges.get(&0x8200_0000).expect("the branch has edges");

        assert_eq!(
            from_branch,
            &[Edge::Taken(0x8200_0100), Edge::Taken(0x8200_0104)]
        );
    }

    #[test]
    fn an_unresolved_branch_still_reports_one_unresolved_edge() {
        let function = switching(&[0x8200_0100], false);

        let edges = function.edges();

        assert_eq!(edges.get(&0x8200_0000), Some(&vec![Edge::Unresolved]));
    }

    /// A branch every one of whose targets lies elsewhere leaves nothing in this
    /// function to point at, which is the same situation as not knowing.
    #[test]
    fn a_branch_leaving_the_function_reports_unresolved() {
        let mut function = switching(&[0x8200_0100], true);
        function.resolved = BTreeMap::from([(0x8200_0000, vec![0x8300_0000])]);

        let edges = function.edges();

        assert_eq!(edges.get(&0x8200_0000), Some(&vec![Edge::Unresolved]));
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

    /// An entry point that is not four byte aligned cannot be an instruction.
    /// Walking from it decodes a stream shifted out of step with the real one,
    /// and claims addresses no instruction sits at, which was found by fuzzing.
    #[test]
    fn an_unaligned_entry_point_seeds_nothing() {
        let words = [encode::addi(3, 4, 1), encode::blr()];
        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0002)
            .code(&words)
            .build();

        let program = analyze(&image, &[]);

        assert_eq!(program.count_from(Origin::EntryPoint), 0);
        assert!(program.claimed_instructions() <= words.len() as u64);
    }

    /// The same for a caller supplied root, which comes from an export table and
    /// is no more trustworthy than the entry point.
    #[test]
    fn an_unaligned_root_seeds_nothing() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::addi(3, 4, 1), encode::blr()])
            .build();

        let program = analyze(&image, &[0x8200_0001]);

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

    /// The shape a real prologue has: read the link register out, then move the
    /// stack pointer down to make room.
    fn prologue() -> [u32; 2] {
        [
            encode::mflr(12),
            encode::stwu(1, 1, encode::back(128) & 0xffff),
        ]
    }

    #[test]
    fn a_function_nothing_calls_is_found_by_its_prologue() {
        let mut words = vec![encode::blr()];
        words.extend_from_slice(&prologue());
        words.push(encode::blr());

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.function_count(), 2);
        assert_eq!(
            program.function_at(0x8200_0004).unwrap().origin,
            Origin::Scanned
        );
    }

    #[test]
    fn a_function_already_reached_is_not_reported_again() {
        let mut words = vec![encode::bl(4)];
        words.extend_from_slice(&prologue());
        words.push(encode::blr());

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0004).unwrap().origin,
            Origin::Called,
            "being called outranks being scanned for"
        );
        assert_eq!(program.count_from(Origin::Scanned), 0);
    }

    /// One signal is not enough. Data will occasionally hold a word that
    /// decodes as something a prologue does, and seeding from it would invent a
    /// function out of a table of constants.
    #[test]
    fn a_single_prologue_signal_does_not_seed_a_function() {
        let words = vec![
            encode::blr(),
            encode::mflr(12),
            encode::addi(3, 4, 1),
            encode::addi(3, 5, 1),
            encode::addi(3, 6, 1),
            encode::blr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Scanned), 0);
    }

    #[test]
    fn a_run_of_constants_seeds_nothing() {
        let mut words = vec![encode::blr()];
        words.extend(std::iter::repeat_n(0x0000_0001u32, 64));

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Scanned), 0);
    }

    #[test]
    fn counts_report_scanned_functions_apart_from_called_ones() {
        let mut words = vec![encode::bl(16), encode::blr()];
        words.extend_from_slice(&prologue());
        words.push(encode::blr());
        words.extend_from_slice(&prologue());
        words.push(encode::blr());

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::EntryPoint), 1);
        assert!(program.count_from(Origin::Called) >= 1);
        assert!(program.count_from(Origin::Scanned) >= 1);
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
