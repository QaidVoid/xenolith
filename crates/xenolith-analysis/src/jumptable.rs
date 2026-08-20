//! Recovering the targets behind an indirect branch.
//!
//! A switch statement compiles to a branch through the count register, and
//! until the table behind it is read that branch is a hole in the control flow
//! graph. Recovery is what turns one unresolved edge into a set of real ones.
//!
//! The shape was read off real code. An index is normalized, compared against a
//! bound, and a conditional branch takes the default when it is out of range.
//! Then a table address is built from a pair of instructions, one entry is
//! loaded from it using the index, and the result is added to a base to give
//! the target. The address is moved into the count register and branched to.
//!
//! Two things follow from that. The bound and the table live in different
//! blocks, because the conditional branch that checks the bound ends one, so
//! recovery has to look back through predecessors. And the values involved are
//! built across several instructions, so they have to be tracked rather than
//! read off any single one.
//!
//! Where a value cannot be established the table is reported unrecovered. It is
//! never guessed at, and in particular entries are never read until one stops
//! looking like an address, because a table followed by something that resembles
//! one would silently produce edges to things that are not code.

use std::collections::BTreeMap;

use xenolith_ppc::{FlowKind, Form, Instruction, Opcode};
use xenolith_xex::Image;

use crate::block::{Block, INSTRUCTION_SIZE, Terminator};
use crate::function::Function;

/// The special purpose register a branch takes its target from.
const COUNT_REGISTER: u32 = 9;

/// How many blocks back recovery will look for the values it needs.
///
/// The bound is one block back in the shape real code uses. A couple more costs
/// nothing and covers a compiler that spills the setup further.
const MAX_PREDECESSORS: usize = 4;

/// Largest table recovery will read.
///
/// A bound far above this is not a switch, it is a misread comparison, and
/// reading it would produce thousands of edges to whatever follows the table.
const MAX_ENTRIES: u32 = 4096;

/// A recovered jump table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpTable {
    /// Address of the branch the table belongs to.
    pub branch: u32,
    /// Register holding the index into the table.
    pub index_register: u8,
    /// Address the table starts at, when one is read.
    pub table: Option<u32>,
    /// Where control goes for each index in range.
    pub targets: Vec<u32>,
    /// Where control goes when the index is out of range.
    pub default: Option<u32>,
}

impl JumpTable {
    /// Returns how many entries the table holds.
    #[must_use]
    pub fn entries(&self) -> usize {
        self.targets.len()
    }
}

/// An entry read out of a table, indexed by a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Load {
    table: u32,
    index: u8,
    width: u8,
    /// Whether the entry is sign extended when loaded.
    signed: bool,
    /// What the entry is multiplied by before it is used.
    ///
    /// Compilers store word offsets divided by four so that a byte covers a
    /// range four times as wide, and scale the entry back up after loading it.
    /// Every dense switch in a real title does this, so an interpretation that
    /// cannot follow the scaling recovers nothing at all.
    scale: u32,
    /// How far apart consecutive entries sit.
    ///
    /// This is the width for a table indexed by a plain count, and the multiplier
    /// for one whose index was scaled up before the load.
    stride: u32,
    /// The register the index was derived from.
    ///
    /// The bound is checked on this one, which is not always the register the
    /// load names.
    root: u8,
}

/// What a register is known to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Value {
    /// Nothing is known about it.
    Unknown,
    /// A value fixed at this point in the code.
    Const(u32),
    /// An entry loaded from a table.
    Entry(Load),
    /// A table entry added to a fixed base, which is how an offset table gives
    /// an address.
    Offset { base: u32, load: Load },
    /// Read out of memory at an address that is not fixed when the title is
    /// built, so what it holds depends on what the program did before.
    ///
    /// Told apart from nothing being known because it is the shape of a call
    /// through a pointer held in an object, which has no table behind it and
    /// never will. Counting one of those as a table that could not be read
    /// makes the recovery look worse than it is and hides the ones that really
    /// were missed.
    Dynamic,
}

/// Tracks what each register holds while reading forward through a run.
#[derive(Debug, Default)]
struct Registers(BTreeMap<u8, Value>);

impl Registers {
    /// Returns what a register holds.
    fn get(&self, register: u8) -> Value {
        self.0.get(&register).copied().unwrap_or(Value::Unknown)
    }

    /// Records what a register holds.
    fn set(&mut self, register: u8, value: Value) {
        self.0.insert(register, value);
    }
}

/// Tracks which register an index was derived from, and by what multiple.
///
/// A compiler routinely bounds an index in one register and then scales it into
/// another to address the table, so the register the load names is often not the
/// register the comparison guards. Following the derivation is what connects the
/// two. Without it the bound cannot be found for most tables, and a table whose
/// bound is unknown has no end.
#[derive(Debug, Default)]
struct Lineage(BTreeMap<u8, (u8, u32)>);

impl Lineage {
    /// Returns the register a value came from and how far it was scaled up.
    fn of(&self, register: u8) -> (u8, u32) {
        self.0.get(&register).copied().unwrap_or((register, 1))
    }

    /// Records that a register now holds another one scaled by `factor`.
    fn derive(&mut self, destination: u8, source: u8, factor: u32) {
        let (root, scale) = self.of(source);
        match scale.checked_mul(factor) {
            Some(scale) => self.0.insert(destination, (root, scale)),
            None => self.0.remove(&destination),
        };
    }

    /// Records that a register no longer holds anything an index came from.
    fn clear(&mut self, destination: u8) {
        self.0.remove(&destination);
    }
}

/// Returns the width in bytes an indexed load reads, and whether it sign extends.
fn indexed_load(opcode: Opcode) -> Option<(u8, bool)> {
    match opcode {
        Opcode::Lbzx => Some((1, false)),
        Opcode::Lhzx => Some((2, false)),
        Opcode::Lhax => Some((2, true)),
        Opcode::Lwzx => Some((4, false)),
        Opcode::Lwax => Some((4, true)),
        _ => None,
    }
}

/// Builds the load an indexed access describes, if the table address is known.
///
/// The address is the sum of the two operands, so either may carry the table and
/// the other the index.
fn indexed(
    registers: &Registers,
    lineage: &Lineage,
    width: u8,
    signed: bool,
    ra: u8,
    rb: u8,
) -> Value {
    let (table, index) = match (registers.get(ra), registers.get(rb)) {
        (Value::Const(table), _) => (table, rb),
        (_, Value::Const(table)) => (table, ra),
        _ => return Value::Dynamic,
    };

    let (root, factor) = lineage.of(index);
    Value::Entry(Load {
        table,
        index,
        width,
        signed,
        scale: 1,
        stride: factor.max(u32::from(width)),
        root,
    })
}

/// Reads one instruction and updates what the registers hold.
fn step(registers: &mut Registers, lineage: &mut Lineage, instruction: Instruction) {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    match instruction.opcode() {
        // Building a constant, either half at a time or in one go.
        Opcode::Addis if ra == 0 => {
            let high = 0u32.wrapping_add_signed(instruction.displacement()) << 16;
            registers.set(rt, Value::Const(high));
            lineage.clear(rt);
        }
        Opcode::Addi if ra == 0 => {
            let value = 0u32.wrapping_add_signed(instruction.displacement());
            registers.set(rt, Value::Const(value));
            lineage.clear(rt);
        }
        Opcode::Addi | Opcode::Addis => {
            let shift = u32::from(instruction.opcode() == Opcode::Addis) * 16;
            let addend = 0u32.wrapping_add_signed(instruction.displacement()) << shift;
            match registers.get(ra) {
                Value::Const(base) => registers.set(rt, Value::Const(base.wrapping_add(addend))),
                _ => registers.set(rt, Value::Unknown),
            }
            // Normalizing an index shifts where it starts but not what it came
            // from, so the bound is still checked on the same register.
            lineage.derive(rt, ra, 1);
        }

        // Loading an entry out of a table whose address is already known.
        opcode if indexed_load(opcode).is_some() => {
            let Some((width, signed)) = indexed_load(opcode) else {
                return;
            };
            registers.set(rt, indexed(registers, lineage, width, signed, ra, rb));
            lineage.clear(rt);
        }

        // A rotate left whose mask keeps everything from the top down to bit 31
        // minus the rotate is a shift left, and a shift left is a multiply. Any
        // other mask discards part of the value, so it is not treated as one.
        //
        // This appears twice in a switch: scaling an entry back up after it was
        // stored divided down, and scaling an index up to address a table of
        // words. The rotates name their destination in the field the other forms
        // use for a source operand.
        Opcode::Rlwinm => {
            let shift = u32::from(instruction.shift_amount());
            let shifts_left =
                instruction.mask_begin() == 0 && u32::from(instruction.mask_end()) + shift == 31;
            let factor = 1u32.checked_shl(shift).unwrap_or(0);

            let value = match registers.get(rt) {
                Value::Entry(load) if shifts_left => match load.scale.checked_mul(factor) {
                    Some(scale) => Value::Entry(Load { scale, ..load }),
                    None => Value::Unknown,
                },
                Value::Const(value) if shifts_left => Value::Const(value.wrapping_mul(factor)),
                _ => Value::Unknown,
            };
            registers.set(ra, value);

            if shifts_left {
                lineage.derive(ra, rt, factor);
            } else {
                lineage.clear(ra);
            }
        }

        // Widening or copying an index leaves both what it is and where it came
        // from alone.
        Opcode::Extsb | Opcode::Extsh | Opcode::Extsw => {
            registers.set(ra, registers.get(rt));
            lineage.derive(ra, rt, 1);
        }
        // A register moved onto itself is how a copy is written.
        Opcode::Or if rt == rb => {
            registers.set(ra, registers.get(rt));
            lineage.derive(ra, rt, 1);
        }
        Opcode::Or => {
            registers.set(ra, Value::Unknown);
            lineage.clear(ra);
        }

        // Adding an entry to a base, which is how an offset becomes an address.
        Opcode::Add => {
            let value = match (registers.get(ra), registers.get(rb)) {
                (Value::Const(base), Value::Entry(load))
                | (Value::Entry(load), Value::Const(base)) => Value::Offset { base, load },
                (Value::Const(left), Value::Const(right)) => Value::Const(left.wrapping_add(right)),
                _ => Value::Unknown,
            };
            registers.set(rt, value);
            lineage.clear(rt);
        }

        // Reading through a base that is not fixed. A virtual call reaches its
        // target with two of these, one for the table in the object and one for
        // the method in the table, and neither address is known before the
        // program runs.
        Opcode::Lwz | Opcode::Ld => {
            let value = match registers.get(ra) {
                Value::Const(_) => Value::Unknown,
                _ => Value::Dynamic,
            };
            registers.set(rt, value);
            lineage.clear(rt);
        }

        // Anything that writes nothing leaves the state alone.
        Opcode::Mtspr | Opcode::Stw | Opcode::Std | Opcode::Cmpl | Opcode::Cmp => {}

        // Anything else makes its destination unknown, which is what keeps a
        // stale value from being trusted after something overwrote it. The
        // rotate and shift forms name their destination where the rest name a
        // source, so writing off the wrong field would leave a dead value in
        // place and let it be believed.
        opcode => {
            let destination = if writes_to_ra(opcode) { ra } else { rt };
            registers.set(destination, Value::Unknown);
            lineage.clear(destination);
        }
    }
}

/// Returns whether an instruction writes the register named by the `ra` field.
///
/// The logical, shift, and rotate instructions take their destination there,
/// which is the reverse of every other form.
fn writes_to_ra(opcode: Opcode) -> bool {
    opcode
        .form()
        .is_some_and(|form| matches!(form, Form::M | Form::MD | Form::MDS | Form::XS))
        || matches!(
            opcode,
            Opcode::And
                | Opcode::Andc
                | Opcode::Nand
                | Opcode::Nor
                | Opcode::Xor
                | Opcode::Eqv
                | Opcode::Orc
                | Opcode::Slw
                | Opcode::Srw
                | Opcode::Sraw
                | Opcode::Srawi
                | Opcode::Sld
                | Opcode::Srd
                | Opcode::Srad
                | Opcode::Cntlzw
                | Opcode::Cntlzd
                | Opcode::Ori
                | Opcode::Oris
                | Opcode::Andi
                | Opcode::Andis
                | Opcode::Xori
                | Opcode::Xoris
        )
}

/// Returns the blocks leading to `block`, nearest last, through single
/// predecessors only.
///
/// Stops at a merge. A value produced in a block reached from more than one
/// place cannot be established by following one of them, and picking a
/// predecessor arbitrarily would produce a confident wrong answer.
fn predecessor_chain<'a>(function: &'a Function, block: &'a Block) -> Vec<&'a Block> {
    let edges = function.edges();
    let mut chain = vec![block];

    for _ in 0..MAX_PREDECESSORS {
        let Some(current) = chain.first().map(|block| block.start) else {
            break;
        };
        let mut sources = function.blocks.iter().filter(|candidate| {
            edges
                .get(&candidate.start)
                .is_some_and(|list| list.iter().any(|edge| edge.target() == Some(current)))
        });

        let (Some(single), None) = (sources.next(), sources.next()) else {
            break;
        };
        chain.insert(0, single);
    }

    chain
}

/// Reads a table entry from the image.
fn entry_at(image: &Image, load: &Load, index: u32) -> Option<u32> {
    let at = load.table.checked_add(index.checked_mul(load.stride)?)?;

    let raw = match load.width {
        1 => u32::from(image.u8(at).ok()?),
        2 => u32::from(image.u16(at).ok()?),
        4 => image.u32(at).ok()?,
        _ => return None,
    };

    if !load.signed {
        return Some(raw);
    }
    let bits = u32::from(load.width) * 8;
    let sign = 1u32 << (bits - 1);
    Some(if raw & sign == 0 {
        raw
    } else {
        raw | !((1u32 << bits) - 1)
    })
}

/// A bound checked on an index, and where control goes when it fails.
#[derive(Debug, Clone, Copy)]
struct Bound {
    /// The register the index was derived from when the check was made.
    root: u8,
    /// The highest index still in range.
    limit: u32,
    /// Where control goes when the index is out of range.
    default: Option<u32>,
}

/// Returns the condition field a compare writes.
fn compared_field(instruction: Instruction) -> u32 {
    u32::from(instruction.rt()) >> 2
}

/// Returns the condition field a conditional branch reads.
fn tested_field(instruction: Instruction) -> u32 {
    instruction.branch_condition_bit() >> 2
}

/// Returns where a conditional branch sends an index that failed its check.
fn default_of(block: &Block) -> Option<u32> {
    match block.terminator {
        Terminator::Transfer {
            kind: FlowKind::Branch,
            target,
            ..
        } => target,
        _ => None,
    }
}

/// Recovers the jump table behind one indirect branch, if it can be read.
///
/// The values and the bound are collected in one forward pass, because the
/// register an index was derived from is only known while walking. Reading the
/// bound afterwards would mean comparing register numbers that no longer refer
/// to the same value.
/// Returns every way into a block, as a chain ending at it.
///
/// One chain when the block has a single predecessor, which is the shape a
/// compiler usually leaves. Where several paths arrive, each is followed
/// separately, because which one was taken is not known here and a bound
/// checked on one of them is not a bound on the others.
fn paths_into<'a>(function: &'a Function, block: &'a Block) -> Vec<Vec<&'a Block>> {
    let edges = function.edges();
    let sources: Vec<&Block> = function
        .blocks
        .iter()
        .filter(|candidate| {
            edges
                .get(&candidate.start)
                .is_some_and(|list| list.iter().any(|edge| edge.target() == Some(block.start)))
        })
        .collect();

    if sources.len() < 2 {
        return vec![predecessor_chain(function, block)];
    }

    sources
        .into_iter()
        .map(|source| {
            let mut chain = predecessor_chain(function, source);
            chain.push(block);
            chain
        })
        .collect()
}

/// Reads a run of blocks, returning what the count register ends up holding and
/// every bound checked along the way.
fn walk(image: &Image, chain: &[&Block]) -> Option<(Value, Vec<Bound>)> {
    let mut registers = Registers::default();
    let mut lineage = Lineage::default();
    let mut target_value = Value::Unknown;
    let mut bounds: Vec<Bound> = Vec::new();

    for step_block in chain {
        let mut compared: BTreeMap<u32, (u8, u32)> = BTreeMap::new();
        let mut address = step_block.start;

        while address < step_block.end {
            let word = image.u32(address).ok()?;
            let instruction = Instruction::decode(word);

            if instruction.opcode() == Opcode::Mtspr && instruction.spr() == COUNT_REGISTER {
                target_value = registers.get(instruction.rt());
            }

            // An unsigned compare against an immediate is what bounds a switch.
            // Which register it came from is only known here, while the walk is
            // at the comparison rather than past it.
            if instruction.opcode() == Opcode::Cmpli {
                compared.insert(
                    compared_field(instruction),
                    (
                        lineage.of(instruction.ra()).0,
                        u32::from(instruction.immediate()),
                    ),
                );
            }

            // Only the comparison writing the field the branch reads is the one
            // guarding the path into the table. A block may compare several
            // things, and accepting any of them would attach a bound to a
            // dispatch through a function pointer, which has none, and report a
            // table that is not there.
            if instruction.flow(address).kind == FlowKind::Branch
                && address.saturating_add(INSTRUCTION_SIZE) == step_block.end
            {
                if let Some(&(root, limit)) = compared.get(&tested_field(instruction)) {
                    bounds.push(Bound {
                        root,
                        limit,
                        default: default_of(step_block),
                    });
                }
            }

            step(&mut registers, &mut lineage, instruction);

            address = address.saturating_add(INSTRUCTION_SIZE);
        }
    }

    Some((target_value, bounds))
}

fn recover_one(image: &Image, function: &Function, block: &Block) -> Option<JumpTable> {
    // Every way in has to give the same answer. A table read on one path and a
    // bound checked on another says nothing about the path actually taken, so
    // the value has to match across all of them and so does the bound, which is
    // checked once the table says which register it is on.
    let walks: Vec<(Value, Vec<Bound>)> = paths_into(function, block)
        .into_iter()
        .map(|chain| walk(image, &chain))
        .collect::<Option<Vec<_>>>()?;
    let (target_value, _) = *walks.first()?;
    if walks.iter().any(|(value, _)| *value != target_value) {
        return None;
    }

    let (base, load) = match target_value {
        Value::Offset { base, load } => (base, load),
        Value::Entry(load) => (0, load),
        _ => return None,
    };

    // A jump table is fixed when the title is built, so it never lives in memory
    // the title can write. A run of addresses that does is an array of function
    // pointers, which has no bound and no default and is not this.
    if image
        .section_at(load.table)
        .is_none_or(|section| section.kind.is_writable())
    {
        return None;
    }

    // The nearest check on each path wins, because an outer one may guard
    // something else, and every path has to have one that agrees.
    let bound = *walks
        .first()?
        .1
        .iter()
        .rev()
        .find(|bound| bound.root == load.root)?;
    for (_, bounds) in &walks {
        let other = bounds.iter().rev().find(|other| other.root == load.root)?;
        if other.limit != bound.limit {
            return None;
        }
    }
    let entries = bound.limit.checked_add(1)?;
    let default = bound.default;
    if entries == 0 || entries > MAX_ENTRIES {
        return None;
    }

    let mut targets = Vec::with_capacity(entries as usize);
    for slot in 0..entries {
        let entry = entry_at(image, &load, slot)?;
        let target = base.wrapping_add(entry.wrapping_mul(load.scale));

        // A target outside executable memory means the table was misread. A
        // partial table is a wrong answer that looks like a right one, so the
        // whole thing is refused.
        if !image
            .section_at(target)
            .is_some_and(|section| section.kind.is_executable())
        {
            return None;
        }
        targets.push(target);
    }

    Some(JumpTable {
        branch: block.end.saturating_sub(INSTRUCTION_SIZE),
        index_register: load.root,
        table: Some(load.table),
        targets,
        default,
    })
}

/// What recovery found across a function.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JumpTables {
    recovered: Vec<JumpTable>,
    unrecovered: Vec<u32>,
    dynamic: Vec<u32>,
}

impl JumpTables {
    /// Returns the tables that were read.
    #[must_use]
    pub fn recovered(&self) -> &[JumpTable] {
        &self.recovered
    }

    /// Returns the addresses of branches whose tables were not read.
    ///
    /// Reported so they can be investigated rather than counted.
    #[must_use]
    pub fn unrecovered(&self) -> &[u32] {
        &self.unrecovered
    }

    /// Returns the addresses of branches that reach a target read out of
    /// memory the program wrote, which is a call through a pointer.
    ///
    /// These have no table behind them and never will, so they are counted
    /// apart from the ones a table was expected from and not found.
    #[must_use]
    pub fn dynamic(&self) -> &[u32] {
        &self.dynamic
    }

    /// Returns how many indirect branches were considered.
    #[must_use]
    pub fn considered(&self) -> usize {
        self.recovered.len() + self.unrecovered.len() + self.dynamic.len()
    }

    /// Merges another function's results into this one.
    pub fn absorb(&mut self, other: Self) {
        self.recovered.extend(other.recovered);
        self.unrecovered.extend(other.unrecovered);
        self.dynamic.extend(other.dynamic);
    }
}

/// Returns whether a branch takes its target from memory the program wrote.
///
/// A call through a pointer held in an object reaches its target with a load
/// whose address is not fixed when the title is built. There is no table behind
/// one and there never will be, so it is not a table that failed to be read.
fn reaches_a_pointer(image: &Image, function: &Function, block: &Block) -> bool {
    paths_into(function, block)
        .into_iter()
        .filter_map(|chain| walk(image, &chain))
        .any(|(value, _)| value == Value::Dynamic)
}

/// Recovers the jump tables of one function.
#[must_use]
pub fn recover(image: &Image, function: &Function) -> JumpTables {
    let mut tables = JumpTables::default();

    for block in &function.blocks {
        // A branch through a register that does not take the link is where a
        // switch lands. One that takes the link is a call, and its target
        // belongs to the call graph rather than to this function's edges.
        let Terminator::Transfer {
            kind: FlowKind::Indirect,
            ..
        } = block.terminator
        else {
            continue;
        };

        let branch = block.end.saturating_sub(INSTRUCTION_SIZE);
        match recover_one(image, function, block) {
            Some(table) => tables.recovered.push(table),
            None if reaches_a_pointer(image, function, block) => tables.dynamic.push(branch),
            None => tables.unrecovered.push(branch),
        }
    }

    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::analyze;
    use crate::testing::{ImageBuilder, encode};

    /// Branch if the condition bit is set, which is how a bound check jumps to
    /// its default.
    const IF_TRUE: u32 = 12;
    /// The bit an unsigned compare sets when the value was greater.
    const GREATER: u32 = 1;

    /// Recovers the only table in an image and returns it.
    fn only_table(image: &Image) -> Option<JumpTable> {
        let program = analyze(image, &[]);
        let mut all = JumpTables::default();
        for function in program.functions() {
            all.absorb(recover(image, function));
        }
        all.recovered.into_iter().next()
    }

    /// A table of byte offsets scaled back up and added to a base, which is the
    /// shape a dense switch compiles to.
    #[test]
    fn recovers_a_table_of_scaled_byte_offsets() {
        // Targets sit at 0x8200002c, 0x82000034, 0x82000038 and 0x8200003c,
        // reached by adding four times the entry to 0x82000020.
        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&[
                encode::cmpli(10, 3),
                encode::bc(IF_TRUE, GREATER, 0x2c),
                encode::addis(12, 0, 0x8200),
                encode::addi(12, 12, 0x40),
                encode::lbzx(0, 12, 10),
                encode::slwi(0, 0, 2),
                encode::addis(12, 0, 0x8200),
                encode::addi(12, 12, 0x20),
                encode::add(12, 12, 0),
                encode::mtctr(12),
                encode::bctr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
            ])
            .code(&[0x0305_0607])
            .build();

        let table = only_table(&image).expect("the table was not recovered");

        assert_eq!(table.branch, 0x8200_0028);
        assert_eq!(table.index_register, 10);
        assert_eq!(table.table, Some(0x8200_0040));
        assert_eq!(table.default, Some(0x8200_0030));
        assert_eq!(
            table.targets,
            [0x8200_002c, 0x8200_0034, 0x8200_0038, 0x8200_003c]
        );
    }

    /// A table of whole addresses reached by scaling the index rather than the
    /// entry. The load names the scaled register, so the bound is only found by
    /// following that register back to the one it came from.
    #[test]
    fn recovers_a_table_of_absolute_addresses() {
        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&[
                encode::cmpli(11, 1),
                encode::bc(IF_TRUE, GREATER, 0x24),
                encode::addis(12, 0, 0x8200),
                encode::addi(12, 12, 0x30),
                encode::slwi(0, 11, 2),
                encode::lwzx(0, 12, 0),
                encode::mtctr(0),
                encode::bctr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
            ])
            .code(&[0x8200_0020, 0x8200_0024])
            .build();

        let table = only_table(&image).expect("the table was not recovered");

        assert_eq!(
            table.index_register, 11,
            "the bound names the unscaled index"
        );
        assert_eq!(table.table, Some(0x8200_0030));
        assert_eq!(table.default, Some(0x8200_0028));
        assert_eq!(table.targets, [0x8200_0020, 0x8200_0024]);
    }

    /// Without a bound there is no end to the table, so nothing is reported
    /// rather than a table that runs on into whatever follows it.
    #[test]
    fn refuses_a_table_with_no_bound() {
        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&[
                encode::addis(12, 0, 0x8200),
                encode::addi(12, 12, 0x20),
                encode::slwi(0, 11, 2),
                encode::lwzx(0, 12, 0),
                encode::mtctr(0),
                encode::bctr(),
                encode::blr(),
                encode::blr(),
            ])
            .code(&[0x8200_0018, 0x8200_001c])
            .build();

        assert_eq!(only_table(&image), None);
    }

    /// One entry pointing outside executable memory means the table was misread,
    /// and a table that is partly right is a wrong answer that looks like a right
    /// one, so the whole thing is refused.
    #[test]
    fn refuses_a_table_whose_entries_leave_executable_memory() {
        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&[
                encode::cmpli(11, 1),
                encode::bc(IF_TRUE, GREATER, 0x24),
                encode::addis(12, 0, 0x8200),
                encode::addi(12, 12, 0x30),
                encode::slwi(0, 11, 2),
                encode::lwzx(0, 12, 0),
                encode::mtctr(0),
                encode::bctr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
                encode::blr(),
            ])
            .code(&[0x8200_0020, 0x9000_0000])
            .build();

        assert_eq!(only_table(&image), None);
    }

    /// A branch whose target cannot be established is reported rather than
    /// dropped, because an unresolved edge is a normal outcome that later work
    /// has to know about.
    #[test]
    fn reports_a_branch_it_could_not_read() {
        let image = ImageBuilder::new(0x8200_0000)
            .entry(0x8200_0000)
            .code(&[encode::mtctr(3), encode::bctr()])
            .build();

        let program = analyze(&image, &[]);
        let mut all = JumpTables::default();
        for function in program.functions() {
            all.absorb(recover(&image, function));
        }

        assert_eq!(all.recovered(), []);
        assert_eq!(all.unrecovered(), [0x8200_0004]);
        assert_eq!(all.considered(), 1);
    }
}
