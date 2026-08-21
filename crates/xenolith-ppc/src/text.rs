//! Rendering a decoded instruction as text.
//!
//! This is for humans reading a disassembly, and it is deliberately off the
//! decode path: nothing here runs unless someone asks for text, so the analysis
//! stages pay nothing for its existence.
//!
//! Operands are printed in the order an assembler writes them, destination
//! first, which for several families is not the order the encoding stores them
//! in. The logical operations and the shifts keep their target in the field
//! every other form uses for a source, and printing those in encoding order
//! said `slw r8, r11, r11` for an instruction that writes r11 and reads r8.
//! Nothing in that tells a reader which register is which, so it was neither
//! honest nor useful.
//!
//! Every rendering here is compared against GNU objdump over both titles' code
//! sections, operand by operand and in order.

use core::fmt;

use crate::form::Form;
use crate::instruction::Instruction;
use crate::table::Opcode;

/// Mnemonics whose displacement addresses memory through a base register.
///
/// These print their operand as a displacement applied to a register rather
/// than as a bare immediate, which is the difference between reading an offset
/// and reading a constant.
const MEMORY_ACCESS: &[&str] = &[
    "lwz", "lwzu", "lbz", "lbzu", "lhz", "lhzu", "lha", "lhau", "stw", "stwu", "stb", "stbu",
    "sth", "sthu", "lmw", "stmw", "lfs", "lfsu", "lfd", "lfdu", "stfs", "stfsu", "stfd", "stfdu",
    "ld", "ldu", "lwa", "std", "stdu",
];

/// An instruction paired with the address it sits at, ready to be printed.
///
/// The address matters because a relative branch is only meaningful once
/// resolved against where it branches from.
#[derive(Debug, Clone, Copy)]
pub struct Rendered {
    instruction: Instruction,
    address: u32,
}

impl Rendered {
    /// Pairs an instruction with its address.
    #[must_use]
    pub const fn new(instruction: Instruction, address: u32) -> Self {
        Self {
            instruction,
            address,
        }
    }
}

/// Returns whether a mnemonic addresses memory through a base register.
fn is_memory_access(mnemonic: &str) -> bool {
    MEMORY_ACCESS.contains(&mnemonic)
}

/// Returns whether a mnemonic reads one source register rather than two.
///
/// These spend the field the rest of their form uses for a second source on
/// nothing, so printing it names a register the instruction never reads.
fn takes_one_source(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "extsb"
            | "extsh"
            | "extsw"
            | "cntlzw"
            | "cntlzd"
            | "popcntb"
            | "neg"
            | "nego"
            | "addze"
            | "addme"
            | "subfze"
            | "subfme"
    )
}

/// Returns whether a mnemonic writes the field the rest of its form reads.
///
/// The logical operations and the shifts store their target where every other
/// instruction of the same form stores a source. Printing them in the order the
/// bits appear names the source as the destination, which reverses what the
/// instruction does.
fn writes_the_second_field(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "and"
            | "andc"
            | "nand"
            | "nor"
            | "or"
            | "orc"
            | "xor"
            | "eqv"
            | "slw"
            | "srw"
            | "sraw"
            | "sld"
            | "srd"
            | "srad"
            | "extsb"
            | "extsh"
            | "extsw"
            | "cntlzw"
            | "cntlzd"
            | "popcntb"
    ) || is_logical_immediate(mnemonic)
}

/// Returns whether a mnemonic names a register in the floating point bank.
fn is_floating(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "lfs"
            | "lfsu"
            | "lfsx"
            | "lfsux"
            | "lfd"
            | "lfdu"
            | "lfdx"
            | "lfdux"
            | "stfs"
            | "stfsu"
            | "stfsx"
            | "stfsux"
            | "stfd"
            | "stfdu"
            | "stfdx"
            | "stfdux"
            | "stfiwx"
    )
}

/// Returns whether a mnemonic names a register in the vector bank.
///
/// These address memory through two general purpose registers and move a whole
/// vector, so only the operand being loaded or stored belongs to that bank.
fn is_vector_access(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "lvx"
            | "lvxl"
            | "lvebx"
            | "lvehx"
            | "lvewx"
            | "lvlx"
            | "lvlxl"
            | "lvrx"
            | "lvrxl"
            | "lvsl"
            | "lvsr"
            | "stvx"
            | "stvxl"
            | "stvebx"
            | "stvehx"
            | "stvewx"
            | "stvlx"
            | "stvlxl"
            | "stvrx"
            | "stvrxl"
    )
}

/// Returns whether a mnemonic reaches memory through a base and an index.
///
/// The float and vector ones are named apart because they also move a register
/// out of another bank. What these share is the base, which reads register zero
/// as the number rather than as a register.
fn is_indexed_access(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "lbzx"
            | "lbzux"
            | "lhzx"
            | "lhzux"
            | "lhax"
            | "lhaux"
            | "lwzx"
            | "lwzux"
            | "lwax"
            | "lwaux"
            | "ldx"
            | "ldux"
            | "lswx"
            | "stbx"
            | "stbux"
            | "sthx"
            | "sthux"
            | "stwx"
            | "stwux"
            | "stdx"
            | "stdux"
            | "stswx"
            | "lwarx"
            | "ldarx"
            | "stwcx."
            | "stdcx."
            | "lhbrx"
            | "lwbrx"
            | "ldbrx"
            | "sthbrx"
            | "stwbrx"
            | "stdbrx"
    )
}

/// Returns whether a mnemonic takes no operand but the register it writes.
///
/// The rest of the form is unused, so printing it names two registers that are
/// not read.
fn takes_no_source(mnemonic: &str) -> bool {
    matches!(mnemonic, "mfcr" | "mfmsr" | "mftb")
}

/// Returns how a base register is written, where zero means the number itself.
///
/// An address formed from a base and an index reads register zero as the
/// literal zero rather than as its contents, so printing it as a register says
/// a register is read that is not.
fn base(ra: u8) -> String {
    if ra == 0 {
        "0".to_owned()
    } else {
        format!("r{ra}")
    }
}

/// Returns whether a mnemonic addresses a cache line rather than a register.
///
/// These name a place in memory with the two fields that form an address and
/// leave the third unused, so printing it names a register nothing reads.
fn addresses_a_cache_line(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "dcbf" | "dcbst" | "dcbt" | "dcbtst" | "dcbz" | "icbi" | "dcbi"
    )
}

/// Writes the operands of a rotate.
///
/// These name their destination in the field every other form uses for a
/// source, and the rest of their operands bound a mask rather than naming
/// registers. The doubleword ones split a six bit field across the word.
fn rotate_operands(f: &mut fmt::Formatter<'_>, instruction: Instruction) -> fmt::Result {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    match instruction.form() {
        Some(Form::M) => {
            let amount = if instruction.opcode() == Opcode::Rlwnm {
                format!("r{rb}")
            } else {
                instruction.shift_amount().to_string()
            };
            write!(
                f,
                " r{ra}, r{rt}, {amount}, {}, {}",
                instruction.mask_begin(),
                instruction.mask_end()
            )
        }
        Some(Form::MDS) => write!(f, " r{ra}, r{rt}, r{rb}, {}", instruction.long_mask_bound()),
        Some(Form::MD) => write!(
            f,
            " r{ra}, r{rt}, {}, {}",
            instruction.long_shift_amount(),
            instruction.long_mask_bound()
        ),
        _ => write!(f, " r{ra}, r{rt}, {}", instruction.long_shift_amount()),
    }
}

/// Writes the operands of a vector instruction that does not take three
/// registers, returning nothing when it does.
///
/// Several of the vector operations spend the field the rest use for a second
/// source on something that is not a register: a constant to splat, a lane to
/// select, or nothing at all.
fn vector_operands(
    f: &mut fmt::Formatter<'_>,
    mnemonic: &str,
    instruction: Instruction,
) -> Option<fmt::Result> {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    if vector_takes_one_source(mnemonic) {
        return Some(write!(f, " v{rt}, v{rb}"));
    }
    // Splatting a constant carries it in that field, signed across five bits.
    if matches!(mnemonic, "vspltisb" | "vspltish" | "vspltisw") {
        let signed = if ra >= 16 {
            i32::from(ra) - 32
        } else {
            i32::from(ra)
        };
        return Some(write!(f, " v{rt}, {signed}"));
    }
    // Splatting a lane names which lane in that same field, unsigned.
    if matches!(mnemonic, "vspltb" | "vsplth" | "vspltw") {
        return Some(write!(f, " v{rt}, v{rb}, {ra}"));
    }
    // Converting between floats and fixed point carries the place of the point
    // in that field, unsigned, so it is a count of bits and not a register.
    if matches!(mnemonic, "vcfux" | "vcfsx" | "vctuxs" | "vctsxs") {
        return Some(write!(f, " v{rt}, v{rb}, {ra}"));
    }

    None
}

/// Returns whether a vector mnemonic reads one register rather than two.
fn vector_takes_one_source(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "vrfim"
            | "vrfin"
            | "vrfip"
            | "vrfiz"
            | "vrefp"
            | "vrsqrtefp"
            | "vexptefp"
            | "vlogefp"
            | "vupkhsb"
            | "vupklsb"
            | "vupkhsh"
            | "vupklsh"
            | "vupkhpx"
            | "vupklpx"
    )
}

/// Returns whether a mnemonic compares and writes a condition field.
fn is_compare(mnemonic: &str) -> bool {
    matches!(mnemonic, "cmp" | "cmpl" | "cmpi" | "cmpli")
}

/// Returns whether a mnemonic combines one condition bit with another.
///
/// These name three bits of the condition register, one written and two read,
/// where the rest of this form names a condition and a branch target. Printing
/// the form's two operands drops the third outright, so the text says which bit
/// was written and only half of what it was written from.
fn combines_condition_bits(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "crand" | "crandc" | "creqv" | "crnand" | "crnor" | "cror" | "crorc" | "crxor"
    )
}

/// Returns whether a mnemonic opens with the conditions it traps on.
///
/// That field is a mask of which comparisons should trap, not a register
/// number. Printing it as a register names one the instruction never reads and
/// loses which comparisons were asked for, which is the whole of what a trap
/// says.
fn traps_on_a_condition(mnemonic: &str) -> bool {
    matches!(mnemonic, "tw" | "twi" | "td" | "tdi")
}

/// Returns whether a mnemonic takes an unsigned immediate.
///
/// The logical operations combine bits rather than counting, so their immediate
/// is a pattern rather than a quantity and is never negative.
fn is_logical_immediate(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "ori" | "oris" | "xori" | "xoris" | "andi." | "andis."
    )
}

/// Returns the value a compare is measured against, written as the field holds
/// it: unsigned for the unsigned compares and signed for the others.
fn compare_operand(mnemonic: &str, instruction: Instruction) -> String {
    if mnemonic == "cmpli" {
        instruction.immediate().to_string()
    } else {
        instruction.displacement().to_string()
    }
}

/// Writes the extended mnemonic for an instruction, if it has a well known one.
///
/// Extended mnemonics are a spelling convention, not different instructions.
/// The decoder always reports the underlying operation, and only the text
/// changes here.
///
/// A spelling that replaces the mnemonic has to carry the variant bits itself,
/// since it is written instead of the name they would otherwise be added to.
/// Dropping the record bit here spells a move that sets a condition field as
/// one that does not, and the two read the same everywhere but the one place
/// the difference matters.
fn extended(f: &mut fmt::Formatter<'_>, instruction: Instruction) -> Option<fmt::Result> {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    match instruction.opcode() {
        // An or of a register with itself moves it.
        Opcode::Or if rt == rb => Some(write!(f, "mr{} r{ra}, r{rt}", suffixes(instruction))),
        // Adding zero to register zero, discarding the result, does nothing.
        Opcode::Ori if rt == 0 && ra == 0 && instruction.immediate() == 0 => Some(write!(f, "nop")),
        // Adding an immediate to nothing loads it.
        Opcode::Addi if ra == 0 => Some(write!(f, "li r{rt}, {}", instruction.displacement())),
        Opcode::Addis if ra == 0 => Some(write!(f, "lis r{rt}, {}", instruction.displacement())),
        _ => None,
    }
}

/// Writes the operands of a branch, resolving a direct target.
fn branch_operands(
    f: &mut fmt::Formatter<'_>,
    instruction: Instruction,
    address: u32,
) -> fmt::Result {
    // A transfer through a register claims no target, and printing nothing is
    // the honest rendering of that.
    match instruction.flow(address).target {
        Some(target) => write!(f, " {target:#010x}"),
        None => Ok(()),
    }
}

/// Writes the operands of an instruction of the general purpose form.
///
/// This form covers more unrelated instructions than any other, and several of
/// them spend a field on something that is not a register at all. Returning
/// nothing leaves the general three register rendering to the caller.
fn general_operands(
    f: &mut fmt::Formatter<'_>,
    mnemonic: &str,
    instruction: Instruction,
) -> Option<fmt::Result> {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    Some(match instruction.form()? {
        Form::X if is_compare(mnemonic) => {
            write!(f, " cr{}, {}, r{ra}, r{rb}", rt >> 2, rt & 1)
        }
        // Comparing floats writes a condition field and reads two of the
        // floating bank, none of which are general purpose registers.
        Form::X if matches!(mnemonic, "fcmpu" | "fcmpo") => {
            write!(f, " cr{}, f{ra}, f{rb}", rt >> 2)
        }
        // The floating conversions and moves take one source and no second
        // operand at all, so printing three general registers names two
        // things that are not there.
        Form::X if mnemonic.starts_with('f') => write!(f, " f{rt}, f{rb}"),
        // The shift count of an immediate arithmetic shift sits where the
        // form otherwise names a register, so printing it as one says a
        // register is read that is not.
        Form::X if mnemonic == "srawi" => write!(f, " r{ra}, r{rt}, {rb}"),
        // A move out of the condition or machine state register, and a read
        // of the timebase, name only what they write.
        Form::X if takes_no_source(mnemonic) => write!(f, " r{rt}"),
        // Moving a special purpose register names it by number, and that
        // number is one field stored in two halves. Printing the halves as
        // registers names two that are not read, and loses which register
        // was meant.
        Form::X if mnemonic == "mfspr" => write!(f, " r{rt}, {}", instruction.spr()),
        Form::X if mnemonic == "mtspr" => write!(f, " {}, r{rt}", instruction.spr()),
        // Ordering takes nothing at all.
        Form::X if matches!(mnemonic, "eieio" | "sync" | "isync") => Ok(()),
        // Moving a whole vector to or from memory addresses it through two
        // general purpose registers, so only the operand moved is a vector.
        Form::X if is_vector_access(mnemonic) => {
            write!(f, " v{rt}, {}, r{rb}", base(ra))
        }
        // The indexed floating loads and stores name the bank they move
        // between, which is not the one their address is built from.
        Form::X if is_floating(mnemonic) => {
            write!(f, " f{rt}, {}, r{rb}", base(ra))
        }
        Form::X | Form::XO if takes_one_source(mnemonic) => {
            if writes_the_second_field(mnemonic) {
                write!(f, " r{ra}, r{rt}")
            } else {
                write!(f, " r{rt}, r{ra}")
            }
        }
        Form::X if addresses_a_cache_line(mnemonic) => write!(f, " {}, r{rb}", base(ra)),
        Form::X if writes_the_second_field(mnemonic) => {
            write!(f, " r{ra}, r{rt}, r{rb}")
        }
        // Everything else of this form that reaches memory reads register
        // zero as the number zero rather than as a register.
        Form::X if is_indexed_access(mnemonic) => {
            write!(f, " r{rt}, {}, r{rb}", base(ra))
        }
        // Writing the machine state register takes one register and a bit
        // saying how much of it to write, not three registers.
        Form::X if matches!(mnemonic, "mtmsr" | "mtmsrd") => {
            write!(f, " r{rt}, {}", ra & 1)
        }
        Form::X if traps_on_a_condition(mnemonic) => write!(f, " {rt}, r{ra}, r{rb}"),
        // Writing the floating status and control register names which of its
        // fields to write, and takes the value from the floating bank. The mask
        // saying which fields, and the two bits either side of it, sit where
        // this form otherwise names registers, so printing three general ones
        // names none of the four things actually there.
        Form::X if mnemonic == "mtfsf" => {
            let word = instruction.word();
            write!(
                f,
                " {}, f{rb}, {}, {}",
                (word >> 17) & 0xff,
                (word >> 25) & 1,
                (word >> 16) & 1
            )
        }
        _ => return None,
    })
}

/// Writes the operands of a floating arithmetic instruction.
///
/// This form carries a third register in a field the others leave empty, and
/// which of the four registers an instruction actually uses depends on the
/// instruction rather than the form. Printing all of them names registers that
/// are never read.
fn floating_operands(
    f: &mut fmt::Formatter<'_>,
    mnemonic: &str,
    instruction: Instruction,
) -> fmt::Result {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());
    let third = (instruction.word() >> 6) & 0x1f;

    match mnemonic {
        "fmul" | "fmuls" => write!(f, " f{rt}, f{ra}, f{third}"),
        "fmadd" | "fmadds" | "fmsub" | "fmsubs" | "fnmadd" | "fnmadds" | "fnmsub" | "fnmsubs"
        | "fsel" => {
            write!(f, " f{rt}, f{ra}, f{third}, f{rb}")
        }
        "fsqrt" | "fsqrts" | "fres" | "frsqrte" => write!(f, " f{rt}, f{rb}"),
        _ => write!(f, " f{rt}, f{ra}, f{rb}"),
    }
}

impl fmt::Display for Rendered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let instruction = self.instruction;

        if instruction.is_unknown() {
            return write!(f, ".long {:#010x}", instruction.word());
        }

        if let Some(result) = extended(f, instruction) {
            return result;
        }

        let mnemonic = instruction.opcode().mnemonic();
        write!(f, "{mnemonic}{}", suffixes(instruction))?;

        let Some(form) = instruction.form() else {
            return Ok(());
        };
        let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

        match form {
            Form::I => branch_operands(f, instruction, self.address),
            // A conditional branch chooses its condition with two fields. Printing
            // only the target loses which condition it was, so a branch taken when
            // equal reads the same as one taken when not.
            Form::B => {
                write!(
                    f,
                    " {}, {},",
                    instruction.branch_condition(),
                    instruction.branch_condition_bit()
                )?;
                branch_operands(f, instruction, self.address)
            }
            Form::SC => Ok(()),
            Form::XL if combines_condition_bits(mnemonic) => {
                write!(f, " {rt}, {ra}, {rb}")
            }
            Form::XL => {
                write!(
                    f,
                    " {}, {}",
                    instruction.branch_condition(),
                    instruction.branch_condition_bit()
                )
            }
            Form::D | Form::DS if is_memory_access(mnemonic) => {
                let bank = if is_floating(mnemonic) { 'f' } else { 'r' };
                write!(
                    f,
                    " {bank}{rt}, {}({})",
                    instruction.displacement(),
                    base(ra)
                )
            }
            // A compare spends the field the others use for a target register
            // on the condition field it writes and the width it compares at.
            // Printing it as a register names something that is not one.
            Form::D if is_compare(mnemonic) => write!(
                f,
                " cr{}, {}, r{ra}, {}",
                rt >> 2,
                rt & 1,
                compare_operand(mnemonic, instruction)
            ),
            // The logical immediates take an unsigned field, so sign extending
            // it reports a negative constant where the instruction has none.
            // They also write the field they would otherwise read.
            Form::D if is_logical_immediate(mnemonic) => {
                write!(f, " r{ra}, r{rt}, {}", instruction.immediate())
            }
            Form::D if traps_on_a_condition(mnemonic) => {
                write!(f, " {rt}, r{ra}, {}", instruction.displacement())
            }
            Form::D | Form::DS => {
                write!(f, " r{rt}, r{ra}, {}", instruction.displacement())
            }
            Form::X | Form::XO if general_operands(f, mnemonic, instruction).is_some() => Ok(()),
            Form::X | Form::XO => write!(f, " r{rt}, r{ra}, r{rb}"),
            // The rotates name their destination where the other forms name a
            // source, and the rest of their operands are mask bounds rather
            // than registers. Only the two that take a variable rotate spend an
            // operand on a register at all.
            Form::M | Form::MDS | Form::MD | Form::XS => rotate_operands(f, instruction),
            Form::A => floating_operands(f, mnemonic, instruction),
            Form::VX if vector_operands(f, mnemonic, instruction).is_some() => Ok(()),
            Form::VX | Form::VC => write!(f, " v{rt}, v{ra}, v{rb}"),
            // Shifting a pair of vectors together takes a byte count where the
            // rest of the form takes a third register, four bits of it rather
            // than five.
            Form::VA if mnemonic == "vsldoi" => write!(
                f,
                " v{rt}, v{ra}, v{rb}, {}",
                (instruction.word() >> 6) & 0xf
            ),
            // A fused multiply names the register it multiplies by before the
            // one it adds, which is the reverse of where the two sit.
            Form::VA if matches!(mnemonic, "vmaddfp" | "vnmsubfp") => write!(
                f,
                " v{rt}, v{ra}, v{}, v{rb}",
                (instruction.word() >> 6) & 0x1f
            ),
            Form::VA => write!(
                f,
                " v{rt}, v{ra}, v{rb}, v{}",
                (instruction.word() >> 6) & 0x1f
            ),
            // The extension's indexed loads and stores address memory through
            // a pair of general purpose registers, so only the destination is a
            // vector register. The rest of the extension forms take three.
            Form::Vx128Ls => write!(f, " v{}, r{ra}, r{rb}", instruction.vector_d()),
            _ => write!(
                f,
                " v{}, v{}, v{}",
                instruction.vector_d(),
                instruction.vector_a(),
                instruction.vector_b()
            ),
        }
    }
}

/// Returns the letters a variant bit adds to a mnemonic.
///
/// The record bit marks an instruction that also updates a condition register
/// field. The link bit marks a branch that records where to come back to, which
/// is the difference between a call and a jump, so leaving it off makes a call
/// read as something it is not.
fn suffixes(instruction: Instruction) -> &'static str {
    let Some(form) = instruction.form() else {
        return "";
    };

    if form.has_record_bit() {
        return if instruction.record_bit() { "." } else { "" };
    }

    match (
        form.has_link_bit() && instruction.link_bit(),
        instruction.absolute_bit(),
    ) {
        (true, true) => "la",
        (true, false) => "l",
        (false, true) => "a",
        (false, false) => "",
    }
}

#[cfg(test)]
mod tests {

    /// The rotates take mask bounds rather than registers, and the doubleword
    /// ones split a six bit field across the instruction. Every case here was
    /// checked against an external disassembler, including one that needs both
    /// split bits set to read correctly.
    #[test]
    fn the_rotates_render_their_bounds_rather_than_registers() {
        let rendered = |word| Instruction::decode(word).render(0).to_string();

        assert_eq!(rendered(0x5400_103a), "rlwinm r0, r0, 2, 0, 29");
        assert_eq!(rendered(0x5c00_103a), "rlwnm r0, r0, r2, 0, 29");
        assert_eq!(rendered(0x5000_0000), "rlwimi r0, r0, 0, 0, 0");
        assert_eq!(rendered(0x7800_0708), "rldic r0, r0, 0, 28");
        assert_eq!(rendered(0x7800_070c), "rldimi r0, r0, 0, 28");
        assert_eq!(rendered(0x7c00_0674), "sradi r0, r0, 0");

        // Both the shift and the mask bound need their sixth bit here, which is
        // stored away from the rest of the field.
        assert_eq!(rendered(0x7800_076a), "rldic r0, r0, 32, 61");
    }
    use super::*;

    fn render(word: u32, address: u32) -> String {
        Rendered::new(Instruction::decode(word), address).to_string()
    }

    #[test]
    fn renders_a_three_register_instruction_in_encoding_order() {
        assert_eq!(render(0x7c62_1a14, 0), "add r3, r2, r3");
    }

    #[test]
    fn renders_the_record_bit_as_a_suffix() {
        assert_eq!(render(0x7c62_1a15, 0), "add. r3, r2, r3");
    }

    #[test]
    fn renders_a_load_as_a_displacement_from_a_base_register() {
        assert_eq!(render(0x8064_0010, 0), "lwz r3, 16(r4)");
    }

    #[test]
    fn renders_a_negative_displacement() {
        assert_eq!(render(0x91c1_ff68, 0), "stw r14, -152(r1)");
    }

    #[test]
    fn renders_an_arithmetic_immediate_as_a_bare_value() {
        assert_eq!(render(0x3864_0010, 0), "addi r3, r4, 16");
    }

    #[test]
    fn resolves_a_relative_branch_to_an_absolute_target() {
        assert_eq!(render(0x4800_0020, 0x8200_1000), "b 0x82001020");
    }

    /// The link bit is the difference between a call and a jump. Leaving it out
    /// of the text makes a call read as something it is not, which is exactly
    /// how a real prologue was misread while writing the analysis stage.
    #[test]
    fn the_link_and_absolute_bits_reach_the_mnemonic() {
        assert_eq!(render(0x4800_0021, 0x8200_1000), "bl 0x82001020");
        assert_eq!(render(0x4800_0022, 0x8200_1000), "ba 0x00000020");
        assert_eq!(render(0x4800_0023, 0x8200_1000), "bla 0x00000020");
    }

    #[test]
    fn a_register_branch_shows_its_link_bit_too() {
        assert_eq!(render(0x4e80_0021, 0), "bclrl 20, 0");
        assert_eq!(render(0x4e80_0421, 0), "bcctrl 20, 0");
    }

    #[test]
    fn an_absolute_branch_ignores_the_instruction_address() {
        assert_eq!(render(0x4800_0022, 0x8200_1000), "ba 0x00000020");
    }

    /// A branch through a register has no target to show, and inventing one
    /// would be worse than showing none.
    #[test]
    fn a_register_branch_claims_no_target() {
        assert_eq!(render(0x4e80_0020, 0x8200_1000), "bclr 20, 0");
        assert_eq!(render(0x4e80_0420, 0x8200_1000), "bcctr 20, 0");
    }

    #[test]
    fn an_unknown_word_renders_as_data() {
        let text = render(0x17ff_ffff, 0);

        assert_eq!(text, ".long 0x17ffffff");
        assert!(!text.contains("<unknown>"), "no mnemonic is invented");
    }

    #[test]
    fn renders_the_common_extended_mnemonics() {
        assert_eq!(render(0x6000_0000, 0), "nop");
        assert_eq!(render(0x3860_0005, 0), "li r3, 5");
        assert_eq!(render(0x7c64_1b78, 0), "mr r4, r3");
    }

    /// The field a trap opens with says which comparisons should trap. Printed
    /// as a register it names one that is never read, and the instruction reads
    /// as trapping on nothing in particular.
    #[test]
    fn a_trap_names_the_conditions_it_traps_on() {
        assert_eq!(render(0x0979_6417, 0), "tdi 11, r25, 25623");
        assert_eq!(render(0x0c09_7d26, 0), "twi 0, r9, 32038");
    }

    /// Combining condition bits reads two and writes one. The form it shares
    /// with the branches has room for two operands, so printing the form's
    /// shape drops the third and loses what was combined with what.
    #[test]
    fn combining_condition_bits_names_all_three() {
        assert_eq!(render(0x4fa6_6242, 0), "creqv 29, 6, 12");
        assert_eq!(render(0x4c8a_9842, 0), "crnor 4, 10, 19");
    }

    /// Writing the floating status and control register names a mask of fields,
    /// a value out of the floating bank, and a bit either side of the mask.
    /// None of the four is a general purpose register.
    #[test]
    fn writing_the_floating_control_register_names_its_mask() {
        assert_eq!(render(0xfc21_c58f, 0), "mtfsf. 16, f24, 0, 1");
    }

    /// A spelling that replaces the mnemonic still has to say that the
    /// instruction sets a condition field, since the oracle this is otherwise
    /// checked against normalizes the record bit away before comparing and so
    /// cannot see it missing.
    #[test]
    fn an_extended_mnemonic_keeps_the_record_bit() {
        assert_eq!(render(0x7c6b_1b79, 0), "mr. r11, r3");
        assert_eq!(render(0x7c6b_1b78, 0), "mr r11, r3");
    }

    /// An extended mnemonic changes the text and nothing else. The decoder
    /// still reports the operation underneath.
    #[test]
    fn an_extended_mnemonic_does_not_change_the_decoded_operation() {
        assert_eq!(Instruction::decode(0x6000_0000).opcode(), Opcode::Ori);
        assert_eq!(Instruction::decode(0x3860_0005).opcode(), Opcode::Addi);
        assert_eq!(Instruction::decode(0x7c64_1b78).opcode(), Opcode::Or);
    }

    /// This family stores its target where every other form of the same shape
    /// stores a source, so the target is printed first even though it is second
    /// in the word. Printing the fields in the order they appear named the
    /// source as the destination, which reverses what the instruction does.
    #[test]
    fn an_encoding_with_no_extended_form_falls_back_to_the_general_one() {
        let word = 0x6083_0005;
        assert_eq!((word >> 21) & 0x1f, 4, "source register field");
        assert_eq!((word >> 16) & 0x1f, 3, "target register field");

        assert_eq!(render(word, 0), "ori r3, r4, 5");
    }

    /// The destination reaches past what standard vector encoding can name,
    /// while the two address operands are ordinary general purpose registers.
    /// Rendering all three as vector registers would misreport the address.
    #[test]
    fn an_extension_load_names_a_wide_destination_and_plain_address_registers() {
        assert_eq!(render(0x100b_60cb, 0), "lvx128 v64, r11, r12");
    }

    #[test]
    fn rendering_never_panics_over_arbitrary_words() {
        let mut state = 0x0bad_c0deu32;
        for _ in 0..50_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let text = render(state, state.rotate_left(11));
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn every_declared_instruction_renders_without_panicking() {
        for word in 0..2048u32 {
            let scaled = word.wrapping_mul(0x0010_0001);
            let text = render(scaled, 0x8200_0000);
            assert!(!text.is_empty());
        }
    }
}
