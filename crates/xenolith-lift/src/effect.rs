//! What an instruction reads and what it writes.
//!
//! This is the part of an instruction's meaning that can be checked mechanically
//! at scale. Whether the arithmetic between two registers is right takes a human
//! or an emulator to say, but whether the right registers were touched can be
//! compared against an independent implementation across a whole title, and a
//! mistake there is the kind that produces code which runs and is quietly wrong.
//!
//! Reads and writes are derived from the instruction's fields rather than from
//! its form. The rotate, shift, and logical operations name their destination in
//! the field every other form uses for a source, so a model that went by form
//! alone would report the wrong register for a large part of the instruction
//! set. Each operation is instead given a shape saying which of its fields are
//! sources and which is the destination.

use xenolith_ppc::{Form, Instruction, Opcode};

/// The condition field a record bit updates.
const RECORD_FIELD: u8 = 0;

/// Somewhere an instruction can read from or write to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Location {
    /// A general purpose register.
    General(u8),
    /// A floating point register.
    Floating(u8),
    /// A vector register, which the console's extension numbers up to 128.
    Vector(u8),
    /// One of the eight condition register fields.
    Condition(u8),
    /// The link register, which a call writes and a return reads.
    Link,
    /// The count register, which a branch through a register reads.
    Count,
    /// The exception register, which carry and overflow live in.
    Exception,
}

/// What one instruction touches.
///
/// Ordered and free of duplicates, so two descriptions of the same instruction
/// can be compared without either side having to normalize first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effect {
    reads: Vec<Location>,
    writes: Vec<Location>,
}

impl Effect {
    /// Returns what the instruction reads.
    #[must_use]
    pub fn reads(&self) -> &[Location] {
        &self.reads
    }

    /// Returns what the instruction writes.
    #[must_use]
    pub fn writes(&self) -> &[Location] {
        &self.writes
    }

    /// Records a read, ignoring one already recorded.
    fn read(&mut self, location: Location) {
        if let Err(at) = self.reads.binary_search(&location) {
            self.reads.insert(at, location);
        }
    }

    /// Records a write, ignoring one already recorded.
    fn write(&mut self, location: Location) {
        if let Err(at) = self.writes.binary_search(&location) {
            self.writes.insert(at, location);
        }
    }
}

/// How an operation's fields map onto what it touches.
///
/// Named for what they do rather than for the encoding form they belong to,
/// because one form covers several shapes: the X form holds both the indexed
/// loads, whose destination is the `rt` field, and the logical operations, whose
/// destination is the `ra` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Writes `rt` from `ra` and `rb`.
    TargetFromBoth,
    /// Writes `rt` from `ra` alone, the rest of the operands being immediate.
    TargetFromFirst,
    /// Writes `ra` from `rt` and `rb`, which is how the logical operations and
    /// the variable shifts are encoded.
    FirstFromTargetAndSecond,
    /// Writes `ra` from `rt` alone, which is how the rotates by a fixed amount
    /// and the sign extensions are encoded.
    FirstFromTarget,
    /// Reads `ra` and `rb` and writes a condition field, which is a compare.
    CompareBoth,
    /// Reads `ra` and an immediate and writes a condition field.
    CompareImmediate,
}

/// Returns how an operation's fields map onto what it touches.
///
/// An operation absent from here has no semantics yet, which is a normal
/// outcome and is reported rather than approximated.
fn shape_of(opcode: Opcode) -> Option<Shape> {
    Some(match opcode {
        Opcode::Add
        | Opcode::Addc
        | Opcode::Adde
        | Opcode::Subf
        | Opcode::Subfc
        | Opcode::Subfe
        | Opcode::Mullw
        | Opcode::Mulld
        | Opcode::Mulhw
        | Opcode::Mulhwu
        | Opcode::Divw
        | Opcode::Divwu
        | Opcode::Divd
        | Opcode::Divdu => Shape::TargetFromBoth,

        Opcode::Addi | Opcode::Addis | Opcode::Addic | Opcode::Mulli | Opcode::Subfic => {
            Shape::TargetFromFirst
        }

        Opcode::And
        | Opcode::Andc
        | Opcode::Nand
        | Opcode::Nor
        | Opcode::Or
        | Opcode::Orc
        | Opcode::Xor
        | Opcode::Eqv
        | Opcode::Slw
        | Opcode::Srw
        | Opcode::Sraw
        | Opcode::Sld
        | Opcode::Srd
        | Opcode::Srad
        | Opcode::Rlwnm => Shape::FirstFromTargetAndSecond,

        Opcode::Rlwinm
        | Opcode::Rlwimi
        | Opcode::Srawi
        | Opcode::Extsb
        | Opcode::Extsh
        | Opcode::Extsw
        | Opcode::Cntlzw
        | Opcode::Cntlzd
        | Opcode::Andi
        | Opcode::Andis
        | Opcode::Ori
        | Opcode::Oris
        | Opcode::Xori
        | Opcode::Xoris => Shape::FirstFromTarget,

        Opcode::Cmp | Opcode::Cmpl => Shape::CompareBoth,
        Opcode::Cmpi | Opcode::Cmpli => Shape::CompareImmediate,

        _ => return None,
    })
}

/// Returns the condition field a compare writes.
///
/// The field number sits above the bit that selects a doubleword comparison,
/// inside the operand position the other forms use for a target register.
fn compared_field(instruction: Instruction) -> u8 {
    instruction.rt() >> 2
}

/// Returns what an instruction reads and writes, if it is modelled.
///
/// Returns `None` for an instruction with no semantics, which is a normal
/// outcome. Nothing approximate is ever returned: a description that is nearly
/// right produces code that is nearly right, and there is no way to tell the
/// difference afterwards.
#[must_use]
pub fn effect_of(instruction: Instruction) -> Option<Effect> {
    // Adding nothing to nothing and discarding it is how doing nothing is
    // written. Reporting the register it names as read and written would be
    // true of the encoding and false of the instruction.
    if is_nothing(instruction) {
        return Some(Effect::default());
    }

    let shape = shape_of(instruction.opcode())?;
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());
    let mut effect = Effect::default();

    match shape {
        Shape::TargetFromBoth => {
            effect.read(Location::General(ra));
            effect.read(Location::General(rb));
            effect.write(Location::General(rt));
        }
        Shape::TargetFromFirst => {
            // Adding to register zero means adding to nothing, which is how a
            // constant is loaded. Reporting a read of r0 there would be wrong.
            if ra != 0 || !loads_a_constant(instruction.opcode()) {
                effect.read(Location::General(ra));
            }
            effect.write(Location::General(rt));
        }
        Shape::FirstFromTargetAndSecond => {
            effect.read(Location::General(rt));
            effect.read(Location::General(rb));
            effect.write(Location::General(ra));
        }
        Shape::FirstFromTarget => {
            effect.read(Location::General(rt));
            effect.write(Location::General(ra));
            // Inserting leaves the bits the mask excludes alone, so the
            // destination is a source as well.
            if instruction.opcode() == Opcode::Rlwimi {
                effect.read(Location::General(ra));
            }
        }
        // A compare copies the summary overflow bit into the field it writes,
        // so it reads the exception register as well as the operands.
        Shape::CompareBoth => {
            effect.read(Location::General(ra));
            effect.read(Location::General(rb));
            effect.read(Location::Exception);
            effect.write(Location::Condition(compared_field(instruction)));
        }
        Shape::CompareImmediate => {
            effect.read(Location::General(ra));
            effect.read(Location::Exception);
            effect.write(Location::Condition(compared_field(instruction)));
        }
    }

    // Both bits are only meaningful where the form carries them. Every other
    // form spends them on something else, so reading them unguarded would have
    // loading a constant update a condition field.
    //
    // Recording a result copies the summary overflow bit into the field, the
    // same way a compare does, so it reads the exception register too.
    let form = instruction.form();
    let records = (form.is_some_and(Form::has_record_bit) && instruction.record_bit())
        || always_records(instruction.opcode());
    if records {
        effect.read(Location::Exception);
        effect.write(Location::Condition(RECORD_FIELD));
    }
    if form.is_some_and(Form::has_overflow_bit) && instruction.overflow_enable() {
        effect.write(Location::Exception);
    }
    // Carrying in or out is carried through the exception register, so an
    // operation that does either touches it whatever its shape.
    if carries(instruction.opcode()) || shifts_out_a_carry(instruction.opcode()) {
        effect.write(Location::Exception);
    }
    if extends_a_carry(instruction.opcode()) {
        effect.read(Location::Exception);
    }

    Some(effect)
}

/// Returns whether an operation records a result whatever its bits say.
///
/// The logical immediates that test rather than compute have no record bit to
/// set, because they always set one. Their spelling carries the dot for that
/// reason rather than as a variant.
fn always_records(opcode: Opcode) -> bool {
    matches!(opcode, Opcode::Andi | Opcode::Andis)
}

/// Returns whether an operation records a carry out into the exception register.
///
/// The arithmetic shifts do, because a right shift of a negative value has to
/// say whether any one bits were shifted out.
fn shifts_out_a_carry(opcode: Opcode) -> bool {
    matches!(opcode, Opcode::Sraw | Opcode::Srawi | Opcode::Srad)
}

/// Returns whether adding to register zero means adding to nothing.
fn loads_a_constant(opcode: Opcode) -> bool {
    matches!(opcode, Opcode::Addi | Opcode::Addis)
}

/// Returns whether an operation records a carry out.
fn carries(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Addc
            | Opcode::Adde
            | Opcode::Addic
            | Opcode::Subfc
            | Opcode::Subfe
            | Opcode::Subfic
    )
}

/// Returns whether an operation reads a carry in.
fn extends_a_carry(opcode: Opcode) -> bool {
    matches!(opcode, Opcode::Adde | Opcode::Subfe)
}

/// Returns whether an instruction does nothing at all.
///
/// Only one spelling means this: adding nothing to nothing and discarding it.
/// Combining a register with itself does write that register, with the value it
/// already held, and is reported as such even where a timing hint is spelled
/// that way.
fn is_nothing(instruction: Instruction) -> bool {
    instruction.opcode() == Opcode::Ori
        && instruction.rt() == 0
        && instruction.ra() == 0
        && instruction.immediate() == 0
}

/// Returns whether an operation has semantics.
#[must_use]
pub fn is_modelled(opcode: Opcode) -> bool {
    shape_of(opcode).is_some()
}

/// Returns every mnemonic that has no semantics yet.
///
/// Compared against the whole instruction table rather than against the
/// instructions some input happened to hold, so what is missing is known rather
/// than found when something reaches it.
pub fn unmodelled() -> impl Iterator<Item = &'static str> {
    Opcode::all()
        .filter(|opcode| !is_modelled(*opcode))
        .map(Opcode::mnemonic)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the effect of a word, or panics naming what was not modelled.
    fn touches(word: u32) -> Effect {
        let instruction = Instruction::decode(word);
        effect_of(instruction)
            .unwrap_or_else(|| panic!("{} is not modelled", instruction.opcode().mnemonic()))
    }

    #[test]
    fn an_add_reads_both_sources_and_writes_its_target() {
        // add r12, r12, r0
        let effect = touches(0x7d8c_0214);

        assert_eq!(
            effect.reads(),
            [Location::General(0), Location::General(12)]
        );
        assert_eq!(effect.writes(), [Location::General(12)]);
    }

    /// The logical operations name their destination in the field every other
    /// form uses for a source, so going by form would report the wrong register.
    #[test]
    fn a_logical_operation_writes_the_register_its_encoding_names() {
        // or r4, r3, r3, which is how a register is copied
        let effect = touches(0x7c64_1b78);

        assert_eq!(effect.reads(), [Location::General(3)]);
        assert_eq!(
            effect.writes(),
            [Location::General(4)],
            "the destination is the ra field, not the rt field"
        );
    }

    /// The same for a rotate, whose remaining operands are not registers at all.
    #[test]
    fn a_rotate_writes_the_register_its_encoding_names() {
        // rlwinm r0, r0, 2, 0, 29
        let effect = touches(0x5400_103a);

        assert_eq!(effect.reads(), [Location::General(0)]);
        assert_eq!(effect.writes(), [Location::General(0)]);

        // rlwinm r8, r4, 2, 0, 29, where source and destination differ
        let effect = touches(0x5488_103a);
        assert_eq!(effect.reads(), [Location::General(4)]);
        assert_eq!(effect.writes(), [Location::General(8)]);
    }

    /// Inserting leaves the bits outside the mask alone, so the destination is
    /// read as well as written.
    #[test]
    fn an_insert_reads_the_register_it_writes() {
        // rlwimi r4, r3, 0, 0, 31
        let effect = touches(0x5064_003e);

        assert!(effect.reads().contains(&Location::General(4)));
        assert!(effect.reads().contains(&Location::General(3)));
        assert_eq!(effect.writes(), [Location::General(4)]);
    }

    #[test]
    fn a_record_bit_adds_a_condition_field_write() {
        // and r11, r11, r10
        let plain = touches(0x7d6b_5038);
        assert_eq!(plain.writes(), [Location::General(11)]);

        // and. r11, r11, r10, the same with the record bit set
        let recording = touches(0x7d6b_5039);
        assert_eq!(
            recording.writes(),
            [Location::General(11), Location::Condition(0)]
        );
    }

    /// A compare copies the summary overflow bit into the field it writes, so
    /// the exception register is a source. Leaving it out was found by checking
    /// the model against an independently produced corpus.
    #[test]
    fn a_compare_writes_the_field_it_names_and_reads_the_exception_register() {
        // cmplwi cr6, r11, 0
        let effect = touches(0x2b0b_0000);

        assert_eq!(effect.reads(), [Location::General(11), Location::Exception]);
        assert_eq!(effect.writes(), [Location::Condition(6)]);
    }

    /// Recording a result copies the same bit the same way.
    #[test]
    fn recording_a_result_reads_the_exception_register() {
        // and. r11, r11, r10
        let effect = touches(0x7d6b_5039);

        assert!(effect.reads().contains(&Location::Exception));
    }

    /// An arithmetic right shift says whether any one bits were shifted out.
    #[test]
    fn an_arithmetic_shift_writes_a_carry() {
        // srawi r11, r11, 2
        let effect = touches(0x7d6b_1670);

        assert!(effect.writes().contains(&Location::Exception));
    }

    /// The logical immediates that test rather than compute always record, and
    /// have no bit to say so because they have no variant that does not.
    #[test]
    fn a_testing_immediate_always_records() {
        // andi. r9, r9, 1
        let effect = touches(0x7129_0001);

        assert!(effect.writes().contains(&Location::Condition(0)));
    }

    /// Adding nothing to nothing and discarding it is how doing nothing is
    /// written, and it touches nothing.
    #[test]
    fn doing_nothing_touches_nothing() {
        let effect = touches(0x6000_0000);

        assert_eq!(effect.reads(), []);
        assert_eq!(effect.writes(), []);
    }

    /// Adding an immediate to register zero adds to nothing, so nothing is read.
    #[test]
    fn loading_a_constant_reads_nothing() {
        // li r4, 5, which is addi r4, r0, 5
        let effect = touches(0x3880_0005);

        assert_eq!(effect.reads(), []);
        assert_eq!(effect.writes(), [Location::General(4)]);
    }

    /// Adding to a register that happens to be numbered zero is a real read,
    /// and only the forms that treat it as nothing may drop it.
    #[test]
    fn adding_to_a_real_register_zero_reads_it() {
        // add r3, r0, r4
        let effect = touches(0x7c60_2214);

        assert!(effect.reads().contains(&Location::General(0)));
    }

    #[test]
    fn an_unmodelled_instruction_admits_it() {
        // lvx128 v64, r11, r12, a console extension load
        assert_eq!(effect_of(Instruction::decode(0x1004_60cb)), None);
    }

    #[test]
    fn what_is_unmodelled_can_be_enumerated() {
        let missing: Vec<&str> = unmodelled().collect();

        assert!(
            !missing.is_empty(),
            "nothing is modelled beyond a subset yet"
        );
        assert!(
            missing.contains(&"lwz"),
            "the loads are not modelled in this group"
        );
        assert!(
            !missing.contains(&"add"),
            "add is modelled and must not be listed"
        );
    }
}
