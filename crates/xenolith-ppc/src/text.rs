//! Rendering a decoded instruction as text.
//!
//! This is for humans reading a disassembly, and it is deliberately off the
//! decode path: nothing here runs unless someone asks for text, so the analysis
//! stages pay nothing for its existence.
//!
//! Operands are printed in encoding order rather than in the order an assembler
//! would accept. The two differ for some families, notably the logical
//! operations, whose assembler syntax names the target register before the
//! source even though the encoding stores them the other way round. Encoding
//! order keeps the output honest about what the bits say, which is what matters
//! when the reason you are reading a disassembly is that something decoded
//! surprisingly.

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

/// Writes the extended mnemonic for an instruction, if it has a well known one.
///
/// Extended mnemonics are a spelling convention, not different instructions.
/// The decoder always reports the underlying operation, and only the text
/// changes here.
fn extended(f: &mut fmt::Formatter<'_>, instruction: Instruction) -> Option<fmt::Result> {
    let (rt, ra, rb) = (instruction.rt(), instruction.ra(), instruction.rb());

    match instruction.opcode() {
        // An or of a register with itself moves it.
        Opcode::Or if rt == rb => Some(write!(f, "mr r{ra}, r{rt}")),
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
            Form::I | Form::B => branch_operands(f, instruction, self.address),
            Form::SC => Ok(()),
            Form::XL => {
                write!(
                    f,
                    " {}, {}",
                    instruction.branch_condition(),
                    instruction.branch_condition_bit()
                )
            }
            Form::D | Form::DS if is_memory_access(mnemonic) => {
                write!(f, " r{rt}, {}(r{ra})", instruction.displacement())
            }
            Form::D | Form::DS => {
                write!(f, " r{rt}, r{ra}, {}", instruction.displacement())
            }
            Form::X | Form::XO => write!(f, " r{rt}, r{ra}, r{rb}"),
            // The rotates name their destination where the other forms name a
            // source, and the rest of their operands are mask bounds rather
            // than registers. Only the two that take a variable rotate spend an
            // operand on a register at all.
            Form::M => {
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
            Form::MDS => write!(f, " r{ra}, r{rt}, r{rb}, {}", instruction.long_mask_bound()),
            Form::MD => write!(
                f,
                " r{ra}, r{rt}, {}, {}",
                instruction.long_shift_amount(),
                instruction.long_mask_bound()
            ),
            Form::XS => write!(f, " r{ra}, r{rt}, {}", instruction.long_shift_amount()),
            Form::A => write!(f, " f{rt}, f{ra}, f{rb}"),
            Form::VX | Form::VC => write!(f, " v{rt}, v{ra}, v{rb}"),
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

    /// An extended mnemonic changes the text and nothing else. The decoder
    /// still reports the operation underneath.
    #[test]
    fn an_extended_mnemonic_does_not_change_the_decoded_operation() {
        assert_eq!(Instruction::decode(0x6000_0000).opcode(), Opcode::Ori);
        assert_eq!(Instruction::decode(0x3860_0005).opcode(), Opcode::Addi);
        assert_eq!(Instruction::decode(0x7c64_1b78).opcode(), Opcode::Or);
    }

    /// Operands print in encoding order, which for this family is the reverse
    /// of assembler syntax: the word stores the source register where an
    /// assembler writes the target. Printing what the bits say is the point.
    #[test]
    fn an_encoding_with_no_extended_form_falls_back_to_the_general_one() {
        let word = 0x6083_0005;
        assert_eq!((word >> 21) & 0x1f, 4, "source register field");
        assert_eq!((word >> 16) & 0x1f, 3, "target register field");

        assert_eq!(render(word, 0), "ori r4, r3, 5");
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
