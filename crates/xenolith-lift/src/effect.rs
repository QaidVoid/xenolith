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
    /// The machine state register, held as storage whose effects are not
    /// modelled.
    Machine,
    /// The floating point status register, held on the same terms.
    FloatingStatus,
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
    /// Writes a floating register from two of them.
    FloatFromBoth,
    /// Writes a floating register from one of them.
    FloatFromOne,
    /// Writes a floating register from three of them, which is a multiply and
    /// an add taken together.
    FloatFromThree,
    /// Writes a floating register from one of them and the third operand.
    FloatMultiply,
    /// Reads two floating registers and writes a condition field.
    FloatCompare,
    /// Touches places its fields name rather than places its form implies.
    Named,
    /// Reads or writes memory at an address built from its operands.
    Memory(Access),
}

/// Where an instruction holds the value it moves, and how it addresses memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Access {
    /// Whether the value travels from memory or to it.
    loading: bool,
    /// Which bank the value lives in.
    bank: Bank,
    /// Whether the address adds a second register rather than a displacement.
    indexed: bool,
    /// Whether the address register is written back with the address used.
    updating: bool,
}

/// Which set of registers a value moves through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bank {
    General,
    Floating,
    Vector,
}

impl Bank {
    /// Returns the place a register of this bank names.
    const fn at(self, register: u8) -> Location {
        match self {
            Self::General => Location::General(register),
            Self::Floating => Location::Floating(register),
            Self::Vector => Location::Vector(register),
        }
    }
}

/// Returns how an operation reaches memory, if it does.
fn access_of(opcode: Opcode) -> Option<Access> {
    let general = |loading, indexed, updating| Access {
        loading,
        bank: Bank::General,
        indexed,
        updating,
    };
    let floating = |loading, indexed, updating| Access {
        loading,
        bank: Bank::Floating,
        indexed,
        updating,
    };

    Some(match opcode {
        Opcode::Lwz | Opcode::Lbz | Opcode::Lhz | Opcode::Lha | Opcode::Ld | Opcode::Lwa => {
            general(true, false, false)
        }
        Opcode::Lwzu | Opcode::Lbzu | Opcode::Lhzu | Opcode::Lhau | Opcode::Ldu => {
            general(true, false, true)
        }
        Opcode::Stw | Opcode::Stb | Opcode::Sth | Opcode::Std => general(false, false, false),
        Opcode::Stwu | Opcode::Stbu | Opcode::Sthu | Opcode::Stdu => general(false, false, true),

        Opcode::Lwzx
        | Opcode::Lbzx
        | Opcode::Lhzx
        | Opcode::Lhax
        | Opcode::Ldx
        | Opcode::Lwax
        | Opcode::Lwbrx
        | Opcode::Lhbrx
        | Opcode::Ldbrx
        | Opcode::Lwarx
        | Opcode::Ldarx => general(true, true, false),
        Opcode::Lwzux
        | Opcode::Lbzux
        | Opcode::Lhzux
        | Opcode::Lhaux
        | Opcode::Ldux
        | Opcode::Lwaux => general(true, true, true),
        Opcode::Stwx
        | Opcode::Stbx
        | Opcode::Sthx
        | Opcode::Stdx
        | Opcode::Stwbrx
        | Opcode::Sthbrx
        | Opcode::Stdbrx
        | Opcode::Stwcx
        | Opcode::Stdcx => general(false, true, false),
        Opcode::Stwux | Opcode::Stbux | Opcode::Sthux | Opcode::Stdux => general(false, true, true),

        Opcode::Lfs | Opcode::Lfd => floating(true, false, false),
        Opcode::Lfsu | Opcode::Lfdu => floating(true, false, true),
        Opcode::Stfs | Opcode::Stfd => floating(false, false, false),
        Opcode::Stfsu | Opcode::Stfdu => floating(false, false, true),
        Opcode::Lfsx | Opcode::Lfdx => floating(true, true, false),
        Opcode::Lfsux | Opcode::Lfdux => floating(true, true, true),
        Opcode::Stfsx | Opcode::Stfdx | Opcode::Stfiwx => floating(false, true, false),

        Opcode::Lvx | Opcode::Lvxl | Opcode::Lvx128 => Access {
            loading: true,
            bank: Bank::Vector,
            indexed: true,
            updating: false,
        },
        Opcode::Stvx | Opcode::Stvxl | Opcode::Stvx128 => Access {
            loading: false,
            bank: Bank::Vector,
            indexed: true,
            updating: false,
        },
        Opcode::Stfsux | Opcode::Stfdux => floating(false, true, true),

        _ => return None,
    })
}

/// Returns how an operation's fields map onto what it touches.
///
/// An operation absent from here has no semantics yet, which is a normal
/// outcome and is reported rather than approximated.
fn shape_of(opcode: Opcode) -> Option<Shape> {
    integer_shape(opcode)
        .or_else(|| floating_shape(opcode))
        .or_else(|| other_shape(opcode))
}

/// Returns how an integer or logical operation's fields map onto what it
/// touches.
fn integer_shape(opcode: Opcode) -> Option<Shape> {
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
        | Opcode::Divdu
        | Opcode::Mulhd
        | Opcode::Mulhdu => Shape::TargetFromBoth,

        // Negating has one source, so it belongs with the immediate arithmetic
        // rather than with the operations that read a second register.
        Opcode::Addi
        | Opcode::Addis
        | Opcode::Addic
        | Opcode::Mulli
        | Opcode::Subfic
        | Opcode::Neg
        | Opcode::AddicRc
        | Opcode::Addze
        | Opcode::Addme
        | Opcode::Subfze
        | Opcode::Subfme => Shape::TargetFromFirst,

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
        | Opcode::Rlwnm
        | Opcode::Rldcl
        | Opcode::Rldcr => Shape::FirstFromTargetAndSecond,

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
        | Opcode::Xoris
        | Opcode::Rldicl
        | Opcode::Rldicr
        | Opcode::Rldic
        | Opcode::Rldimi
        | Opcode::Sradi => Shape::FirstFromTarget,

        opcode if access_of(opcode).is_some() => Shape::Memory(access_of(opcode)?),

        _ => return None,
    })
}

/// Returns how a floating point operation's fields map onto what it touches.
fn floating_shape(opcode: Opcode) -> Option<Shape> {
    Some(match opcode {
        // Floating point arithmetic reads and writes the floating bank, and
        // is otherwise shaped like the integer arithmetic it mirrors.
        Opcode::Fadd
        | Opcode::Fadds
        | Opcode::Fsub
        | Opcode::Fsubs
        | Opcode::Fdiv
        | Opcode::Fdivs => Shape::FloatFromBoth,
        Opcode::Fmr
        | Opcode::Fneg
        | Opcode::Fabs
        | Opcode::Fnabs
        | Opcode::Frsp
        | Opcode::Fcfid
        | Opcode::Fctiw
        | Opcode::Fctiwz
        | Opcode::Fctid
        | Opcode::Fctidz
        | Opcode::Fsqrt
        | Opcode::Fsqrts
        | Opcode::Fres
        | Opcode::Frsqrte => Shape::FloatFromOne,
        Opcode::Fmadd
        | Opcode::Fmadds
        | Opcode::Fmsub
        | Opcode::Fmsubs
        | Opcode::Fnmadd
        | Opcode::Fnmadds
        | Opcode::Fnmsub
        | Opcode::Fnmsubs
        | Opcode::Fsel => Shape::FloatFromThree,
        // Multiplying takes its second operand from the field the fused forms
        // put their third in, not from the one the rest of the arithmetic uses.
        Opcode::Fmul | Opcode::Fmuls => Shape::FloatMultiply,
        Opcode::Fcmpu | Opcode::Fcmpo => Shape::FloatCompare,

        _ => return None,
    })
}

/// Returns how the remaining operations map onto what they touch.
fn other_shape(opcode: Opcode) -> Option<Shape> {
    Some(match opcode {
        Opcode::Cmp | Opcode::Cmpl => Shape::CompareBoth,
        Opcode::Cmpi | Opcode::Cmpli => Shape::CompareImmediate,

        // These name what they touch in a way no shape describes, so each is
        // handled where the shapes are applied rather than being given one.
        Opcode::Mtspr
        | Opcode::Mfspr
        | Opcode::Mftb
        | Opcode::Mfcr
        | Opcode::Mtcrf
        | Opcode::Mcrf
        | Opcode::Mfmsr
        | Opcode::Mtmsr
        | Opcode::Mtmsrd
        | Opcode::Mffs
        | Opcode::Mtfsf
        | Opcode::Mtfsfi
        | Opcode::Crand
        | Opcode::Crandc
        | Opcode::Cror
        | Opcode::Crorc
        | Opcode::Crxor
        | Opcode::Crnand
        | Opcode::Crnor
        | Opcode::Creqv
        | Opcode::B
        | Opcode::Bc
        | Opcode::Bclr
        | Opcode::Bcctr
        | Opcode::Sync
        | Opcode::Isync
        | Opcode::Eieio
        | Opcode::Dcbz
        | Opcode::Dcbt
        | Opcode::Dcbtst
        | Opcode::Dcbf
        | Opcode::Dcbst
        | Opcode::Icbi
        | Opcode::Tw
        | Opcode::Twi
        | Opcode::Td
        | Opcode::Tdi => Shape::Named,

        _ => return None,
    })
}

/// Returns what an instruction that names its own places touches.
///
/// Some operations do not read a register named by a field at all. A branch
/// reads a condition field chosen by its condition operand, a move to a special
/// register writes whichever one its field selects, and a trap reads its
/// operands to decide whether to leave the function entirely.
fn named_effect(instruction: Instruction, effect: &mut Effect) {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    match instruction.opcode() {
        Opcode::Mtspr => {
            effect.read(Location::General(rt));
            if let Some(register) = special(instruction.spr()) {
                effect.write(register);
            }
        }
        Opcode::Mfspr | Opcode::Mftb => {
            if let Some(register) = special(instruction.spr()) {
                effect.read(register);
            }
            effect.write(Location::General(rt));
        }
        // Reading the condition register reads every field of it.
        Opcode::Mfcr => {
            for field in 0..8 {
                effect.read(Location::Condition(field));
            }
            effect.write(Location::General(rt));
        }
        // Writing it writes the fields the mask selects.
        Opcode::Mtcrf => {
            effect.read(Location::General(rt));
            for field in 0..8 {
                if instruction.word() & (1 << (19 - u32::from(field))) != 0 {
                    effect.write(Location::Condition(field));
                }
            }
        }
        Opcode::Mcrf => {
            effect.read(Location::Condition(ra >> 2));
            effect.write(Location::Condition(rt >> 2));
        }
        Opcode::Mfmsr => {
            effect.read(Location::Machine);
            effect.write(Location::General(rt));
        }
        Opcode::Mtmsr | Opcode::Mtmsrd => {
            effect.read(Location::General(rt));
            effect.write(Location::Machine);
        }
        Opcode::Mffs => {
            effect.read(Location::FloatingStatus);
            effect.write(Location::Floating(rt));
        }
        // Writing selected fields leaves the rest, so the register is read as
        // well as written whatever the mask says.
        Opcode::Mtfsf => {
            effect.read(Location::Floating(rb));
            effect.read(Location::FloatingStatus);
            effect.write(Location::FloatingStatus);
        }
        Opcode::Mtfsfi => {
            effect.read(Location::FloatingStatus);
            effect.write(Location::FloatingStatus);
        }
        // The condition register logicals name single bits, and a bit belongs
        // to the field holding it.
        Opcode::Crand
        | Opcode::Crandc
        | Opcode::Cror
        | Opcode::Crorc
        | Opcode::Crxor
        | Opcode::Crnand
        | Opcode::Crnor
        | Opcode::Creqv => {
            effect.read(Location::Condition(ra >> 2));
            effect.read(Location::Condition(rb >> 2));
            effect.write(Location::Condition(rt >> 2));
        }
        Opcode::Bc | Opcode::Bclr | Opcode::Bcctr | Opcode::B => {
            branch_effect(instruction, effect);
        }
        // Zeroing a block builds an address and writes through it, so it reads
        // the registers that address is built from. The cache hints beside it
        // build one too and then do nothing with it, and since no code is
        // emitted for them, saying they touch nothing keeps the model and the
        // code agreeing about the same instruction.
        Opcode::Dcbz => {
            if ra != 0 {
                effect.read(Location::General(ra));
            }
            effect.read(Location::General(rb));
        }
        Opcode::Tw | Opcode::Td => {
            effect.read(Location::General(ra));
            effect.read(Location::General(rb));
        }
        Opcode::Twi | Opcode::Tdi => effect.read(Location::General(ra)),
        // Ordering touches no register this model describes.
        _ => {}
    }
}

/// Records what a branch consults and what taking it leaves behind.
///
/// A branch reads the field its condition names, unless it is taken whatever
/// the condition register holds. One that decrements the count register reads
/// and writes it. One that takes the link writes it.
fn branch_effect(instruction: Instruction, effect: &mut Effect) {
    if instruction.opcode() != Opcode::B {
        let condition = instruction.branch_condition();
        if condition & 0b1_0000 == 0 {
            let bit = u8::try_from(instruction.branch_condition_bit()).unwrap_or(0);
            effect.read(Location::Condition(bit >> 2));
        }
        if condition & 0b0_0100 == 0 {
            effect.read(Location::Count);
            effect.write(Location::Count);
        }
        if instruction.opcode() == Opcode::Bclr {
            effect.read(Location::Link);
        }
        if instruction.opcode() == Opcode::Bcctr {
            effect.read(Location::Count);
        }
    }

    if instruction.link_bit() {
        effect.write(Location::Link);
    }
}

/// Returns the register a special purpose register number names.
///
/// Only the three this model describes are named. Anything else is a register
/// the model has nothing to say about, and saying nothing is the answer.
fn special(number: u32) -> Option<Location> {
    match number {
        1 => Some(Location::Exception),
        8 => Some(Location::Link),
        9 => Some(Location::Count),
        _ => None,
    }
}

/// Returns the third operand of a fused arithmetic instruction.
///
/// It sits where no other form puts an operand, which is why the forms that
/// take three are the only ones that read it.
fn third_operand(instruction: Instruction) -> u8 {
    u8::try_from((instruction.word() >> 6) & 0x1f).unwrap_or(0)
}

/// Returns the condition field a compare writes.
///
/// The field number sits above the bit that selects a doubleword comparison,
/// inside the operand position the other forms use for a target register.
fn compared_field(instruction: Instruction) -> u8 {
    instruction.rt() >> 2
}

/// Writes what a shape says an instruction touches.
fn apply(shape: Shape, instruction: Instruction, effect: &mut Effect) {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

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
            if matches!(instruction.opcode(), Opcode::Rlwimi | Opcode::Rldimi) {
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
        Shape::FloatFromBoth => {
            effect.read(Location::Floating(ra));
            effect.read(Location::Floating(rb));
            effect.write(Location::Floating(rt));
        }
        Shape::FloatFromOne => {
            effect.read(Location::Floating(rb));
            effect.write(Location::Floating(rt));
        }
        Shape::FloatFromThree => {
            effect.read(Location::Floating(ra));
            effect.read(Location::Floating(rb));
            effect.read(Location::Floating(third_operand(instruction)));
            effect.write(Location::Floating(rt));
        }
        Shape::FloatMultiply => {
            effect.read(Location::Floating(ra));
            effect.read(Location::Floating(third_operand(instruction)));
            effect.write(Location::Floating(rt));
        }
        Shape::FloatCompare => {
            effect.read(Location::Floating(ra));
            effect.read(Location::Floating(rb));
            effect.write(Location::Condition(compared_field(instruction)));
        }
        Shape::Named => named_effect(instruction, effect),
        Shape::Memory(access) => {
            // An address built on register zero is built on nothing, the same
            // way adding to it is. Only the forms that write the address
            // register back always have one, since writing back to nothing
            // would have nowhere to go.
            if ra != 0 || access.updating {
                effect.read(Location::General(ra));
            }
            if access.indexed {
                effect.read(Location::General(rb));
            }
            // The console's extension numbers its vector registers past what
            // the field the other banks use can hold, and reads the number from
            // a wider one split across the word. The instructions it extends
            // still use the ordinary field.
            let extended = instruction.form().is_some_and(Form::is_console_extension);
            let moved = if access.bank == Bank::Vector && extended {
                access.bank.at(instruction.vector_d())
            } else {
                access.bank.at(rt)
            };
            if access.loading {
                effect.write(moved);
            } else {
                effect.read(moved);
            }
            if access.updating {
                effect.write(Location::General(ra));
            }
        }
    }
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
    let mut effect = Effect::default();

    apply(shape, instruction, &mut effect);

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
    matches!(opcode, Opcode::Andi | Opcode::Andis | Opcode::AddicRc)
}

/// Returns whether an operation records a carry out into the exception register.
///
/// The arithmetic shifts do, because a right shift of a negative value has to
/// say whether any one bits were shifted out.
fn shifts_out_a_carry(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Sraw | Opcode::Srawi | Opcode::Srad | Opcode::Sradi
    )
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
            | Opcode::AddicRc
            | Opcode::Addze
            | Opcode::Addme
            | Opcode::Subfc
            | Opcode::Subfe
            | Opcode::Subfic
            | Opcode::Subfze
            | Opcode::Subfme
    )
}

/// Returns whether an operation reads a carry in.
fn extends_a_carry(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Adde
            | Opcode::Subfe
            | Opcode::Addze
            | Opcode::Addme
            | Opcode::Subfze
            | Opcode::Subfme
    )
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
        // vmrghw v0, v0, v0, a vector arithmetic instruction
        assert_eq!(effect_of(Instruction::decode(0x1000_008c)), None);
    }

    #[test]
    fn what_is_unmodelled_can_be_enumerated() {
        let missing: Vec<&str> = unmodelled().collect();

        assert!(
            !missing.is_empty(),
            "nothing is modelled beyond a subset yet"
        );
        assert!(
            missing.contains(&"vmrghw"),
            "the vector arithmetic is not modelled yet"
        );
        assert!(
            !missing.contains(&"add"),
            "add is modelled and must not be listed"
        );
    }
}
