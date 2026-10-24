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
    /// Nothing reaches it directly and it has no shape of its own, and it was
    /// found because the run of words it sits in accounts for itself.
    ///
    /// Counted apart from the one above for the same reason that one is counted
    /// apart from a call: the evidence is different. That one recognized what a
    /// function does at its start. This one recognized a hole that nothing can
    /// be entered into and nothing leaves.
    Surrounded,
    /// Nothing reaches it directly, and it was found by recognizing the shape
    /// of a function prologue.
    ///
    /// A function reached by a call is known to be one. This one is believed to
    /// be one, which is why the two are counted apart.
    Scanned,
    /// The image's own record of its functions names it.
    ///
    /// This is the strongest of the passes that go looking, because it is not a
    /// reading of what the words are: the list was written by whatever built
    /// the image, for its own purposes, and it says where a function is
    /// outright.
    Recorded,
    /// Something wrote down where it is: its address sits in a run of others
    /// like it, which a title walks and calls through.
    ///
    /// Nothing calls one of these, so discovery cannot reach it, but a table of
    /// addresses is a stronger thing to have found it by than any shape.
    Tabled,
    /// The code builds its address out of two halves and keeps it somewhere,
    /// to be called through later.
    ///
    /// A title registering a handler does this: the address is in no table and
    /// nothing calls it by name, and the only trace of it is the pair of
    /// instructions that puts it together.
    Constructed,
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
            Self::Recorded => "recorded",
            Self::Tabled => "tabled",
            Self::Surrounded => "surrounded",
            Self::Constructed => "constructed",
        }
    }
}

/// Every origin a function can have.
///
/// The report prints one line per entry, and a test asserts they sum to the
/// function count, so an origin added without a line here fails rather than
/// going unmentioned.
pub const ORIGINS: &[Origin] = &[
    Origin::EntryPoint,
    Origin::Root,
    Origin::Called,
    Origin::Scanned,
    Origin::Recorded,
    Origin::Tabled,
    Origin::Constructed,
    Origin::Surrounded,
];

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

/// Returns whether an address holds an import thunk the loader never patched.
///
/// A container records where each import is called through, and leaves behind
/// four words there: two describing which library and ordinal is wanted, and
/// then a branch through the count register for whatever writes the address in
/// to use. What is written in is the console's business and never happened
/// here, so the description is still there to read.
///
/// The shape is exact. Across a title's whole image every one of the addresses
/// the import records name matches it, and nothing that is not one of them
/// does.
fn is_import_thunk(image: &Image, address: u32) -> bool {
    let (Ok(library), Ok(ordinal), Ok(into_count), Ok(branch)) = (
        image.u32(address),
        image.u32(address.wrapping_add(4)),
        image.u32(address.wrapping_add(8)),
        image.u32(address.wrapping_add(12)),
    ) else {
        return false;
    };

    library >> 24 == 0x01
        && ordinal >> 24 == 0x02
        && library & 0x00ff_ffff == ordinal & 0x00ff_ffff
        && into_count == 0x7d69_03a6
        && branch == 0x4e80_0420
}

/// Returns the branches into an import thunk that nothing claimed.
///
/// A call to an import too far away to reach goes through one of these, and
/// nothing in the image names it: what names it is a pointer the title writes
/// while it runs. So neither scanning for a prologue nor reading the image
/// finds one, and a program reaching one has nowhere to go.
///
/// What makes this safe to claim is the target. An unpatched import thunk is
/// four exact words, and a branch to one is a branch to an import.
/// Returns the addresses of import thunks and of the branches into them.
///
/// A call to an import too far away to reach goes through one of these, and
/// nothing in the image names it: what names it is a pointer the title writes
/// while it runs. So neither scanning for a prologue nor reading the image
/// finds one, and a program reaching one has nowhere to go.
///
/// What makes this safe to claim is the shape. An unpatched import thunk is
/// four exact words, and a branch to one is a branch to an import. A thunk is
/// found both from a branch that names it and from its own words, because a
/// title reaches some of them through a pointer that is written while it runs,
/// and those are the ones a branch scan alone never sees. The words are four
/// exact encodings, so recognizing them is the same evidence as being named.
fn scan_for_import_branches(image: &Image, walked: &BTreeMap<u32, Vec<Block>>) -> Vec<u32> {
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
    for section in image.sections() {
        if !section.kind.is_executable() {
            continue;
        }
        let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
        let mut at = section.start;
        while at < end {
            let address = at;
            at = at.saturating_add(4);
            if claimed.contains(&address) {
                continue;
            }
            if is_import_thunk(image, address) {
                found.push(address);
                continue;
            }
            let Ok(word) = image.u32(address) else {
                continue;
            };
            let instruction = Instruction::decode(word);
            if instruction.opcode() != Opcode::B || instruction.link_bit() {
                continue;
            }
            if instruction
                .flow(address)
                .target
                .is_some_and(|target| is_import_thunk(image, target))
            {
                found.push(address);
            }
        }
    }

    found
}

/// Returns addresses the image points at that jump straight to a function.
///
/// A trampoline is one unconditional branch and nothing else, which is what a
/// linker leaves behind when a call has to reach further than its displacement
/// allows. It has no prologue, so scanning for one never finds it, and nothing
/// calls it directly, so discovery never reaches it. What names it is a word
/// somewhere in the image, and a program that reaches one at runtime through
/// that word finds nothing there and stops.
///
/// Three things have to hold together before one is claimed: the word is an
/// address in executable memory, the instruction there is an unconditional
/// branch that does not take the link, and what it branches to is already known
/// to be a function. Any one alone is weak and all three together are not
/// something data does by accident.
fn scan_for_trampolines(image: &Image, walked: &BTreeMap<u32, Vec<Block>>) -> Vec<u32> {
    let starts: BTreeSet<u32> = walked.keys().copied().collect();
    let mut found = Vec::new();

    for section in image.sections() {
        let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
        let mut at = section.start;
        while at < end {
            at = at.saturating_add(4);
            let Ok(word) = image.u32(at.wrapping_sub(4)) else {
                continue;
            };
            let candidate = word;
            if candidate % 4 != 0 || starts.contains(&candidate) || !is_executable(image, candidate)
            {
                continue;
            }
            let Ok(first) = image.u32(candidate) else {
                continue;
            };
            let instruction = Instruction::decode(first);
            if instruction.opcode() != Opcode::B || instruction.link_bit() {
                continue;
            }
            // Where it goes has to be code, and need not already be known: a
            // trampoline claimed here is a tail call to its target, so
            // discovery walks on to it. That is how the function behind one
            // gets found, and several of them keep their frame in the red zone
            // and so leave no prologue to have found them by.
            let flow = instruction.flow(candidate);
            if flow
                .target
                .is_some_and(|target| starts.contains(&target) || is_executable(image, target))
            {
                found.push(candidate);
            }
        }
    }

    found
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

/// How many addresses in a row make a table of them.
///
/// One word that happens to fall inside the code is nothing. A run of them, each
/// four byte aligned and each landing on something that decodes, is a table a
/// compiler wrote: the list a title walks at startup to run what has to be run
/// before main, or the methods of a class. Four is short enough to catch the
/// small ones and long enough that data does not reach it by chance.
const TABLE_RUN: usize = 4;

/// Returns the addresses a table of function pointers names.
///
/// Nothing calls what one of these holds. A title walks the table and calls
/// through it, so the address appears nowhere in the code, only in the data, and
/// the functions it names are written without prologues as often as not. What
/// makes them safe to claim is not what they look like but that something wrote
/// down where they are: a run of addresses in a row, all landing on code, is not
/// a thing data does by accident.
///
/// Nothing distinguishes a table from the code around it by the page it sits
/// on: this console's images mark the whole executable range read only rather
/// than as code, so a table and the instructions it names share a section. What
/// separates them is that the run has to name addresses, and an instruction
/// names one only when its immediate happens to be one.
///
/// A table is as often a list of records as a list of addresses: a name, the
/// function that answers to it, and something more, over and over. There the
/// addresses are not in a row but one to a record, at the same place in each.
/// So a run is looked for at every spacing up to [`TABLE_STRIDE`] bytes, and a
/// run at a wider spacing is held to the same length as one in a row. One title
/// keeps the handlers for its commands this way, and the ones that touch no
/// stack were found by nothing else, so calling one stopped the run.
fn scan_for_tables(image: &Image, walked: &BTreeMap<u32, Vec<Block>>) -> Vec<u32> {
    let starts: BTreeSet<u32> = walked.keys().copied().collect();
    let claimed = claimed_instructions(walked);
    let mut found = Vec::new();

    for section in image.sections() {
        let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
        let words = (end.saturating_sub(section.start) / INSTRUCTION_SIZE) as usize;
        let named: Vec<Option<u32>> = (0..words)
            .map(|word| {
                let at = section.start + u32::try_from(word).unwrap_or(0) * INSTRUCTION_SIZE;
                image
                    .u32(at)
                    .ok()
                    .filter(|target| names_code(image, *target))
            })
            .collect();

        for stride in 1..=(TABLE_STRIDE / INSTRUCTION_SIZE) as usize {
            for phase in 0..stride {
                let mut run: Vec<u32> = Vec::new();
                for target in named.iter().skip(phase).step_by(stride) {
                    match target {
                        Some(target) => run.push(*target),
                        None => claim_run(&mut found, &mut run, &starts, &claimed, TABLE_RUN),
                    }
                }
                claim_run(&mut found, &mut run, &starts, &claimed, TABLE_RUN);
            }
        }
    }

    found.sort_unstable();
    found.dedup();
    found
}

/// The widest spacing between the addresses of a table of records, in bytes.
///
/// Eight words, which holds a record of a name, a handler and six words more.
/// Wider records are rarer, and a wider spacing lets a run of four reach across
/// more data that happens to hold addresses.
const TABLE_STRIDE: u32 = 32;

/// Returns every instruction address inside a block that was already walked.
fn claimed_instructions(walked: &BTreeMap<u32, Vec<Block>>) -> BTreeSet<u32> {
    let mut claimed = BTreeSet::new();
    for blocks in walked.values() {
        for block in blocks {
            let mut address = block.start;
            while address < block.end {
                claimed.insert(address);
                address = address.saturating_add(INSTRUCTION_SIZE);
            }
        }
    }
    claimed
}

/// Moves a finished run into the found list when it is long enough.
fn claim_run(
    found: &mut Vec<u32>,
    run: &mut Vec<u32>,
    starts: &BTreeSet<u32>,
    claimed: &BTreeSet<u32>,
    least: usize,
) {
    if run.len() >= least {
        found.extend(
            run.iter()
                .copied()
                .filter(|one| !starts.contains(one) && !claimed.contains(one)),
        );
    }
    run.clear();
}

/// How far after the instruction holding the top half of an address the one
/// adding the bottom half is looked for. A compiler puts the two close together
/// but not always next to each other: one title fills a table of ten addresses
/// with every top half loaded first and the bottom halves added fifteen
/// instructions on.
const HALVES_APART: u32 = 16;

/// Returns the addresses the code puts together out of two halves.
///
/// A title that registers a handler builds the handler's address with one
/// instruction loading the top half and another adding the bottom half, and
/// keeps it somewhere to call through. Nothing else records where that function
/// is. An address put together this way is claimed when it lands on code, is
/// not inside anything already walked, and is not the tail of whatever precedes
/// it: the same three things a table's addresses are held to, and the last of
/// them keeps an address built to point inside a function, as a label's is,
/// from being taken for a function of its own.
///
/// Inside something already walked is allowed where the walk only ever branched
/// there, so that a block starts at it and nothing falls into it. That is a
/// function another one hands over to with a plain branch, which a walk takes
/// for part of whatever branched, and one title calls such a function through
/// the address it built and nothing else.
///
/// Most addresses built this way are of data, and data placed just after a
/// function decodes as often as not. So what is found must also look like a
/// function in its own right: open with a prologue, read as one whole leaf, or
/// begin a run of code nothing has claimed that ends the way a function ends.
/// The last is what a handler that finishes by branching to another function
/// looks like, and what a table of addresses placed after a switch does not:
/// its words decode, but they run on into the code that follows the table,
/// which is already claimed, before anything ends.
fn scan_for_constructed(
    image: &Image,
    helpers: &Helpers,
    walked: &BTreeMap<u32, Vec<Block>>,
) -> Vec<u32> {
    let starts: BTreeSet<u32> = walked.keys().copied().collect();
    let claimed = claimed_instructions(walked);
    let branched_to: BTreeSet<u32> = walked.values().flatten().map(|block| block.start).collect();
    let mut found = BTreeSet::new();

    for section in image.executable_sections() {
        let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
        let mut at = section.start;
        while at < end {
            let Some(next) = at.checked_add(INSTRUCTION_SIZE) else {
                break;
            };
            let top = image.u32(at).map(Instruction::decode).ok();
            if let Some(top) = top.filter(|one| one.opcode() == Opcode::Addis && one.ra() == 0) {
                let high = 0u32.wrapping_add_signed(top.displacement()) << 16;
                let mut after = next;
                for _ in 0..HALVES_APART {
                    if after >= end {
                        break;
                    }
                    let Ok(word) = image.u32(after) else {
                        break;
                    };
                    let one = Instruction::decode(word);
                    let low = match one.opcode() {
                        Opcode::Addi if one.ra() == top.rt() => {
                            Some(high.wrapping_add_signed(one.displacement()))
                        }
                        Opcode::Ori if one.rs() == top.rt() => {
                            Some(high | (0u32.wrapping_add_signed(one.displacement()) & 0xffff))
                        }
                        _ => None,
                    };
                    if let Some(target) = low {
                        if names_code(image, target)
                            && !starts.contains(&target)
                            && (!claimed.contains(&target) || branched_to.contains(&target))
                            && nothing_falls_into(image, target)
                            && (prologue_signals(image, target, helpers) >= PROLOGUE_SIGNALS
                                || reads_as_a_leaf(image, target, &starts, &starts).is_some()
                                || ends_before_anything_claimed(image, target, &claimed))
                        {
                            found.insert(target);
                        }
                        break;
                    }
                    after = after.saturating_add(INSTRUCTION_SIZE);
                }
            }
            at = next;
        }
    }

    found.into_iter().collect()
}

/// The most instructions followed from an address the code builds before it
/// is given up on.
///
/// Longer than [`LEAF_LIMIT`], because the address being built is itself the
/// evidence: a title building one at the front of a run of code nothing else
/// has claimed means a function starts there however long it is. One title
/// passes the address of a leaf of two hundred and seventy instructions this
/// way and calls it through what it was handed.
const CONSTRUCTED_LIMIT: u32 = 4096;

/// Whether the words from an address are code nothing has claimed, up to one
/// that ends a function: a return, or a branch or a branch through a register
/// that does not fall through.
fn ends_before_anything_claimed(image: &Image, begin: u32, claimed: &BTreeSet<u32>) -> bool {
    let mut at = begin;
    for _ in 0..CONSTRUCTED_LIMIT {
        if claimed.contains(&at) {
            return false;
        }
        let Ok(word) = image.u32(at) else {
            return false;
        };
        let instruction = Instruction::decode(word);
        if instruction.is_unknown() {
            return false;
        }
        let flow = instruction.flow(at);
        if !flow.falls_through
            && matches!(
                flow.kind,
                FlowKind::Return | FlowKind::Branch | FlowKind::Indirect
            )
        {
            return true;
        }
        let Some(next) = at.checked_add(INSTRUCTION_SIZE) else {
            return false;
        };
        at = next;
    }
    false
}

/// Records each address not already recorded, as found by `origin`, and says
/// whether any was new.
fn record_new(origins: &mut BTreeMap<u32, Origin>, found: Vec<u32>, origin: Origin) -> bool {
    let mut added = false;
    for address in found {
        if let std::collections::btree_map::Entry::Vacant(slot) = origins.entry(address) {
            slot.insert(origin);
            added = true;
        }
    }
    added
}

/// How many pairs in a row make the record an image keeps of its own functions.
const RECORD_RUN: usize = 8;

/// Returns the addresses an image's own record of its functions names.
///
/// An image built for this console carries a list of where its functions are,
/// kept for unwinding a stack rather than for anything here, and it is the one
/// place a function's address is written down whether or not anything calls it.
/// The list is pairs: an address, and a word of flags that is not one.
///
/// It is found by that shape rather than by name, because the shape is what says
/// it is the list. Addresses that only ever climb, each landing on something
/// that decodes, each followed by a word that is not an address, is not a thing
/// data arrives at by chance. What the list leaves out is the functions that
/// never touch the stack, which have nothing to unwind and so need no entry, and
/// those are what the other passes here are for.
fn scan_for_recorded(image: &Image, walked: &BTreeMap<u32, Vec<Block>>) -> Vec<u32> {
    let starts: BTreeSet<u32> = walked.keys().copied().collect();
    let mut found = Vec::new();

    for section in image.sections() {
        let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
        let mut run: Vec<u32> = Vec::new();
        let mut at = section.start;
        let mut last = 0;

        while at < end {
            let flags_at = at.checked_add(INSTRUCTION_SIZE).filter(|next| *next < end);
            let pair = image
                .u32(at)
                .ok()
                .zip(flags_at.and_then(|next| image.u32(next).ok()))
                .filter(|&(begin, flags)| {
                    names_code(image, begin) && !names_code(image, flags) && begin > last
                });

            if let Some((begin, _)) = pair {
                last = begin;
                run.push(begin);
                at = at.saturating_add(INSTRUCTION_SIZE * 2);
            } else {
                claim_run(&mut found, &mut run, &starts, &BTreeSet::new(), RECORD_RUN);
                last = 0;
                at = at.saturating_add(INSTRUCTION_SIZE);
            }
        }

        claim_run(&mut found, &mut run, &starts, &BTreeSet::new(), RECORD_RUN);
    }

    found
}

/// Returns whether a word is an address of something that decodes.
fn names_code(image: &Image, word: u32) -> bool {
    word % INSTRUCTION_SIZE == 0
        && is_executable(image, word)
        && image
            .u32(word)
            .is_ok_and(|first| !Instruction::decode(first).is_unknown())
}

/// The register a branch through one reads its address out of.
const COUNT_REGISTER: u32 = 9;

/// How far back the setting up in front of a hand over is followed.
const HANDOVER_WINDOW: u32 = 8;

/// Returns the addresses of stubs that hand over through a pointer.
///
/// A stub writes the address it was given into the count register and branches
/// through it, so the branch is indirect and nothing about its target is known
/// here. What makes it a function is the pair of words: a move into the count
/// register followed immediately by a branch through it, with only loads in
/// front of them.
///
/// The loads are what tell one of these from the other thing that ends the same
/// way. A switch reaches its branch through the same two words, but it works
/// the address out with arithmetic on an index, which decodes as an indexed
/// load rather than one of these. Allowing anything else here would put a
/// function in the middle of every switch in the image.
fn scan_for_handovers(
    image: &Image,
    helpers: &Helpers,
    walked: &BTreeMap<u32, Vec<Block>>,
) -> Vec<u32> {
    let claimed = claimed_instructions(walked);
    let mut found = Vec::new();

    for section in image.executable_sections() {
        let end = u32::try_from(section.end()).unwrap_or(u32::MAX);
        let mut at = section.start;

        while at < end {
            let Some(next) = at.checked_add(INSTRUCTION_SIZE) else {
                break;
            };

            let hands_over = image.u32(at).is_ok_and(|word| {
                let one = Instruction::decode(word);
                one.opcode() == Opcode::Mtspr && one.spr() == COUNT_REGISTER
            }) && image.u32(next).is_ok_and(|word| {
                let one = Instruction::decode(word);
                !one.is_unknown() && one.flow(next).kind == FlowKind::Indirect
            });

            if !hands_over {
                at = next;
                continue;
            }

            let mut begin = at;
            for _ in 0..HANDOVER_WINDOW {
                let Some(before) = begin.checked_sub(INSTRUCTION_SIZE) else {
                    break;
                };
                if before < section.start {
                    break;
                }
                let Ok(word) = image.u32(before) else {
                    break;
                };
                let one = Instruction::decode(word);
                if !matches!(one.opcode(), Opcode::Lwz | Opcode::Ld) {
                    break;
                }
                begin = before;
            }

            if begin < at
                && !claimed.contains(&begin)
                && helpers.containing(begin).is_none()
                && nothing_falls_into(image, begin)
            {
                found.push(begin);
            }

            at = next;
        }
    }

    found
}

/// The most instructions a function found by its surroundings may be.
///
/// What makes one of these safe to claim is not its length but that it accounts
/// for itself: nothing falls into its start, nothing leaves it but a return or a
/// call that does not come back, and no branch inside it points past its end.
/// The bound is here so that a run which never satisfies that stops being
/// followed rather than running to the end of the section, and it is set well
/// past the length of anything a compiler emits without a prologue.
const LEAF_LIMIT: u32 = 256;

/// Returns whether the word before an address ends whatever came before it.
///
/// A function found by its surroundings has to be shown not to be the tail of
/// the one in front of it. Nothing falls into it where what precedes it is
/// padding, a return, or a branch that does not come back.
fn nothing_falls_into(image: &Image, address: u32) -> bool {
    let Some(before) = address.checked_sub(INSTRUCTION_SIZE) else {
        return false;
    };
    let Ok(word) = image.u32(before) else {
        return false;
    };
    if word == 0 {
        return true;
    }

    let instruction = Instruction::decode(word);
    if instruction.is_unknown() {
        return false;
    }

    let flow = instruction.flow(before);
    // A conditional transfer falls through, so the word after it is reached and
    // nothing begins there. A branch through a register does not come back, and
    // one function beginning at the word after another one's is how a run of
    // these stubs is laid out, with nothing between them to say where one ends.
    !flow.falls_through
        && matches!(
            flow.kind,
            FlowKind::Return | FlowKind::Branch | FlowKind::Indirect
        )
}

/// Returns how far a function starting here runs, or nothing where the words do
/// not read as one whole function.
///
/// The test is that the run accounts for itself. Every word decodes, nothing
/// calls out or branches away, and the last thing it does is return with no
/// branch still pointing past it. A run satisfying all of that is a function
/// whichever way it is read: there is no way into the middle of it from
/// outside, and no way out of it but the return at the end.
///
/// The run grows to cover what its own branches point at, which is what a walk
/// over blocks does. Stopping at the first return instead would cut a function
/// in half wherever a comparison branches forward over one arm.
///
/// This is what finds the ones no shape gives away. A leaf that keeps its frame
/// in the red zone writes no prologue, and one reached only through a pointer is
/// called from nowhere, so neither discovery nor the prologue scan sees it. What
/// is left is the hole it leaves between two functions that were found.
fn reads_as_a_leaf(
    image: &Image,
    begin: u32,
    known: &BTreeSet<u32>,
    any: &BTreeSet<u32>,
) -> Option<u32> {
    if !nothing_falls_into(image, begin) {
        return None;
    }

    let mut at = begin;
    let mut furthest = begin;
    let mut does_something = false;

    for _ in 0..LEAF_LIMIT {
        // A function known to be one, rather than believed to be, is where this
        // run stops being a candidate: walking through it would swallow it.
        if at != begin && known.contains(&at) {
            return None;
        }

        let word = image.u32(at).ok()?;
        let instruction = Instruction::decode(word);
        if instruction.is_unknown() {
            return None;
        }

        let flow = instruction.flow(at);
        let after = at.checked_add(INSTRUCTION_SIZE)?;

        match flow.kind {
            // Nothing may leave. A call and a branch through a register are
            // both reasons to say no: either would make this the middle of
            // something rather than the whole of it.
            FlowKind::Call | FlowKind::Indirect => return None,
            FlowKind::Branch => {
                does_something = true;
                let target = flow.target?;

                // A branch to a function known to be one is a call that does
                // not come back, which is a way for a function to end as much
                // as a return is. Anything else has to stay inside.
                //
                // Whether this is an ending is asked of every function start
                // found so far, not only the certain ones: where a branch goes
                // is the question, and a start believed to be one is still
                // somewhere a branch can hand over to. Stopping the walk is the
                // other question, and that one is only asked of the certain
                // ones, because walking through a believed start is how the
                // function under it gets found.
                if any.contains(&target) && (target < begin || target >= after) {
                    // Only an unconditional transfer ends the run here. One
                    // that falls through leaves the word after it reachable,
                    // so the tail belongs to this function rather than to a
                    // function of its own.
                    if !flow.falls_through && after > furthest {
                        return Some(after);
                    }
                } else {
                    if target < begin {
                        return None;
                    }
                    // An unconditional branch leaves the words after it
                    // unreachable, so a run that continues past one is not a
                    // run at all.
                    if !flow.falls_through {
                        return (after > furthest).then_some(after);
                    }
                    furthest = furthest.max(target);
                }
            }
            FlowKind::Continue => {
                if let Some(target) = flow.target {
                    if target < begin {
                        return None;
                    }
                    furthest = furthest.max(target);
                }
                does_something = true;
            }
            FlowKind::Return => {
                // A return with nothing still pointing past it is the end. A
                // conditional return falls through, so the word after it is
                // still reachable.
                if !flow.falls_through && after > furthest {
                    return does_something.then_some(after);
                }
            }
        }

        at = after;
        furthest = furthest.max(at);
    }

    None
}

/// Returns the addresses of functions that only the gaps around them give away.
///
/// Runs of words nothing claimed sit between the functions that were found.
/// Most are padding or data. One that reads as a whole function, entered
/// nowhere but at its start and left nowhere but at its return, is one.
fn scan_for_leaves(
    image: &Image,
    helpers: &Helpers,
    walked: &BTreeMap<u32, Vec<Block>>,
    known: &BTreeSet<u32>,
    any: &BTreeSet<u32>,
) -> Vec<u32> {
    let claimed = claimed_instructions(walked);
    let mut found = Vec::new();

    for section in image.executable_sections() {
        let mut address = section.start;

        while u64::from(address) < section.end() {
            let next = address.saturating_add(INSTRUCTION_SIZE);
            if next <= address {
                break;
            }

            // One branch with padding on both sides of it.
            //
            // A compiler puts these where a call has to reach further than a
            // call can, and lays them out one to a slot with the slack between
            // them left empty. Nothing about the branch says which function it
            // belongs to, so a walk that ran past a return can pick it up and
            // claim it, and that claim is the reason it would otherwise be
            // missed. Padding on both sides is what says the claim is wrong:
            // nothing reaches this from the word in front of it.
            let alone = image.u32(address.wrapping_sub(INSTRUCTION_SIZE)).ok() == Some(0)
                && image.u32(next).ok() == Some(0)
                && image.u32(address).is_ok_and(|word| {
                    let one = Instruction::decode(word);
                    one.opcode() == Opcode::B
                        && !one.link_bit()
                        && one
                            .flow(address)
                            .target
                            .is_some_and(|to| is_executable(image, to))
                });

            if alone && !walked.contains_key(&address) {
                found.push(address);
                address = next;
                continue;
            }

            if claimed.contains(&address)
                || helpers.containing(address).is_some()
                || image.u32(address).ok() == Some(0)
            {
                address = next;
                continue;
            }

            if let Some(end) = reads_as_a_leaf(image, address, known, any) {
                found.push(address);
                address = end.max(next);
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

        added |= record_new(
            &mut origins,
            scan_for_prologues(image, &helpers, &walked),
            Origin::Scanned,
        );

        added |= record_new(
            &mut origins,
            scan_for_import_branches(image, &walked),
            Origin::Scanned,
        );

        added |= record_new(
            &mut origins,
            scan_for_handovers(image, &helpers, &walked),
            Origin::Surrounded,
        );

        added |= record_new(
            &mut origins,
            scan_for_recorded(image, &walked),
            Origin::Recorded,
        );

        added |= record_new(
            &mut origins,
            scan_for_tables(image, &walked),
            Origin::Tabled,
        );

        added |= record_new(
            &mut origins,
            scan_for_constructed(image, &helpers, &walked),
            Origin::Constructed,
        );

        added |= record_new(
            &mut origins,
            scan_for_trampolines(image, &walked),
            Origin::Scanned,
        );

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

        // What a hole in the code gives away is looked for only once nothing
        // else has anything left to add.
        //
        // It is the weakest evidence here: a call proves a function, a prologue
        // shows one, and this only says that a run of words could not be
        // anything else. Running it first would let it claim addresses that
        // discovery was about to reach and prove.
        if !added {
            let known: BTreeSet<u32> = origins
                .iter()
                .filter(|(_, origin)| {
                    matches!(
                        origin,
                        Origin::Called | Origin::Root | Origin::EntryPoint | Origin::Recorded
                    )
                })
                .map(|(address, _)| *address)
                .collect();

            let any: BTreeSet<u32> = origins.keys().copied().collect();

            for address in scan_for_leaves(image, &helpers, &walked, &known, &any) {
                if let std::collections::btree_map::Entry::Vacant(slot) = origins.entry(address) {
                    slot.insert(Origin::Surrounded);
                    added = true;
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

    /// A word of flags, which is what the second of each pair holds. It is not
    /// an address of anything that decodes.
    const FLAGS: u32 = 0x4000_1204;

    /// Every function has exactly one origin, so the counts the report prints
    /// add up to the function count. An origin added without a line in the
    /// report would otherwise go unmentioned and the numbers would not agree.
    #[test]
    fn origins_sum() {
        let words = vec![
            encode::blr(),
            0,
            encode::addi(3, 4, 1),
            encode::addi(3, 3, 1),
            encode::blr(),
        ];
        let program = analyze(&image(&words), &[]);

        let counted: usize = ORIGINS.iter().map(|one| program.count_from(*one)).sum();

        assert_eq!(counted, program.function_count());
        assert_eq!(
            ORIGINS.len(),
            8,
            "the report prints one line per origin, so this is the number of lines"
        );
    }

    /// Builds the pairs an image's record of its functions is made of.
    fn record_of(addresses: &[u32]) -> Vec<u32> {
        let mut words = Vec::new();
        for &address in addresses {
            words.push(address);
            words.push(FLAGS);
        }
        words
    }

    /// The image's own record names its functions whether or not anything
    /// calls them, which is what finds the ones handed over as a pointer.
    #[test]
    fn the_record_an_image_keeps_names_its_functions() {
        let mut words = vec![encode::blr(); 8];
        let named: Vec<u32> = (0..8).map(|which| 0x8200_0000 + which * 4).collect();
        words.extend(record_of(&named));

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0010).map(|one| one.origin),
            Some(Origin::Recorded)
        );
    }

    /// A few pairs are not the record, and reading them as one would put a
    /// function wherever data held a number beside another.
    #[test]
    fn too_short_a_run_of_pairs_is_not_the_record() {
        let mut words = vec![encode::blr(); 8];
        let named: Vec<u32> = (0..7).map(|which| 0x8200_0000 + which * 4).collect();
        words.extend(record_of(&named));

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Recorded), 0);
    }

    /// A run of addresses in a row is a table, and what it names are functions
    /// however little they look like one.
    #[test]
    fn a_table_of_addresses_names_functions() {
        let words = vec![
            encode::blr(),
            0,
            encode::addi(3, 4, 1),
            encode::blr(),
            0x8200_0008,
            0x8200_0008,
            0x8200_0008,
            0x8200_0008,
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0008).map(|one| one.origin),
            Some(Origin::Tabled)
        );
    }

    /// An address the code builds out of two halves names a function, however
    /// little it looks like one.
    #[test]
    fn an_address_built_from_two_halves_names_a_function() {
        let words = vec![
            encode::addis(3, 0, 0x8200),
            encode::addi(3, 3, 0x10),
            encode::blr(),
            encode::blr(),
            encode::addi(4, 5, 1),
            encode::blr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0010).map(|one| one.origin),
            Some(Origin::Constructed)
        );
    }

    /// One built for a handler that calls something and returns, with no
    /// prologue and so no leaf, still names a function: the words from it are
    /// code nothing has claimed, up to the return.
    #[test]
    fn an_address_built_for_a_handler_that_calls_names_a_function() {
        let words = vec![
            encode::addis(3, 0, 0x8200),
            encode::addi(3, 3, 0x10),
            encode::blr(),
            encode::blr(),
            encode::bl(0x3ff_fff0),
            encode::blr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0010).map(|one| one.origin),
            Some(Origin::Constructed)
        );
    }

    /// One built for a leaf that another function hands over to with a plain
    /// branch names a function, though the walk of the one that branches took
    /// the leaf for its own. The halves are far apart, as a compiler leaves
    /// them when it loads every top half of a table first.
    #[test]
    fn an_address_built_for_a_leaf_branched_to_names_a_function() {
        let mut words = vec![encode::addis(8, 0, 0x8200)];
        words.extend(std::iter::repeat_n(encode::addi(4, 5, 1), 14));
        words.extend([
            encode::addi(10, 8, 0x58),
            encode::std(10, 11, 0),
            encode::bl(0x8),
            encode::blr(),
            encode::addi(4, 0, 0),
            encode::b(0x8),
            encode::blr(),
            encode::addi(3, 3, 1),
            encode::blr(),
        ]);

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0058).map(|one| one.origin),
            Some(Origin::Constructed)
        );
    }

    /// One built to point inside a function already walked names a place in
    /// it, not a function of its own.
    #[test]
    fn an_address_built_into_a_function_names_nothing_new() {
        let words = vec![
            encode::addis(3, 0, 0x8200),
            encode::addi(3, 3, 0x8),
            encode::addi(4, 5, 1),
            encode::blr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Constructed), 0);
    }

    /// A list of records naming one function each, at the same place in every
    /// record, is a table too.
    #[test]
    fn a_table_of_records_names_functions() {
        let mut words = vec![encode::blr(), 0, encode::addi(3, 4, 1), encode::blr()];
        for _ in 0..4 {
            words.extend([0x1234_5678, 0x8200_0008, 0, 0x0000_0010]);
        }

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0008).map(|one| one.origin),
            Some(Origin::Tabled)
        );
    }

    /// Three records are not a table, however regular, for the same reason
    /// three addresses in a row are not.
    #[test]
    fn three_records_are_not_a_table() {
        let mut words = vec![encode::blr(), 0, encode::addi(3, 4, 1), encode::blr()];
        for _ in 0..3 {
            words.extend([0x1234_5678, 0x8200_0008, 0, 0x0000_0010]);
        }

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Tabled), 0);
    }

    /// A run of addresses that lands inside a function already walked names
    /// places inside it, not functions of their own.
    #[test]
    fn a_table_pointing_into_a_function_claims_nothing_new() {
        // Eight instructions, so the entry point covers 0x82000000 to 0x8200001c,
        // then a run of addresses pointing into that body.
        let mut words = vec![encode::addi(3, 4, 1); 8];
        words.extend([0x8200_0004, 0x8200_0008, 0x8200_000c, 0x8200_0010]);

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Tabled), 0);
        assert_eq!(program.function_count(), 1);
    }

    /// One address that happens to fall inside the code is not a table, and
    /// treating it as one would put a function wherever data held a number.
    #[test]
    fn too_few_addresses_in_a_row_are_not_a_table() {
        let words = vec![
            encode::blr(),
            0,
            encode::addi(3, 4, 1),
            encode::blr(),
            0x8200_0008,
            0x8200_0008,
            0x8200_0008,
            0,
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Tabled), 0);
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

        assert_eq!(function.edges()[&0x8200_0000], []);
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
        assert_eq!(function.unreachable_blocks(), Vec::<&Block>::new());
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
    /// and claims addresses no instruction sits at.
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

    /// A leaf with nothing of its own to recognize it by is found by the hole
    /// it sits in.
    ///
    /// This is the shape a real title's comparison functions have: no prologue
    /// because they never touch the stack, and no caller because they are
    /// handed to something as a pointer. Nothing about the words themselves
    /// says function. What says it is that the run is entered nowhere and left
    /// nowhere but its return.
    #[test]
    fn a_leaf_nothing_calls_is_found_by_what_surrounds_it() {
        let words = vec![
            encode::blr(),
            0,
            encode::addi(3, 4, 1),
            encode::addi(3, 3, 1),
            encode::blr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Surrounded), 1);
        assert_eq!(
            program.function_at(0x8200_0008).map(|one| one.origin),
            Some(Origin::Surrounded)
        );
    }

    /// A stub that writes the count register and branches through it hands over
    /// to an address nothing here knows, so the pair of words is what says it is
    /// a function.
    #[test]
    fn a_handover_through_the_count_register_is_a_function() {
        let words = vec![
            encode::blr(),
            0,
            encode::lwz(3, 4, 8),
            encode::mtctr(3),
            encode::bctr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(
            program.function_at(0x8200_0008).map(|one| one.origin),
            Some(Origin::Surrounded)
        );
    }

    /// A run that something falls into is the tail of what came before it
    /// rather than a function, and claiming it would cut a function in two.
    #[test]
    fn a_run_something_falls_into_is_not_a_function_of_its_own() {
        // A function that ends, then a run of words the word before it falls
        // through into. The run is the tail of something rather than a start.
        let words = vec![
            encode::addi(3, 4, 1),
            encode::addi(3, 3, 1),
            encode::addi(3, 2, 1),
            encode::addi(3, 1, 1),
            encode::blr(),
        ];

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Surrounded), 0);
    }

    /// Nothing but a return is slack a compiler left, and a function is put at
    /// each of them by a rule that does not ask for one thing to be done.
    #[test]
    fn a_stray_return_is_not_a_function() {
        // Nothing but a return, in a gap that nothing falls into. It accounts
        // for itself but does nothing, and a function is put here only by a
        // rule that does not ask for one thing to be done.
        let words = vec![encode::blr(), 0, encode::blr(), 0, encode::blr()];

        let program = analyze(&image(&words), &[]);

        assert_eq!(program.count_from(Origin::Surrounded), 0);
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
