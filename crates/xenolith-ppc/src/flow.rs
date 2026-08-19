//! What an instruction does to control flow.
//!
//! The decoder is the only place that knows the encoding, so it is the right
//! place to answer this. Having the analysis stage re-derive it from opcodes
//! would put the same knowledge in two places, and the facts involved are
//! encoding level ones: the link bit is what separates a call from a jump, the
//! absolute bit is what decides whether a displacement is added to the current
//! address, and the branch condition field is what decides whether the next
//! instruction is still reachable.

use crate::instruction::Instruction;
use crate::table::Opcode;

/// What kind of control transfer an instruction performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowKind {
    /// Execution continues at the following instruction.
    Continue,
    /// Transfers control without expecting to come back.
    Branch,
    /// Transfers control and expects to come back, so the following
    /// instruction is where it comes back to.
    Call,
    /// Returns to whoever called the function this instruction sits in.
    Return,
    /// Transfers control through a register, so nothing about the target can be
    /// known from this instruction alone.
    Indirect,
}

/// How an instruction affects control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flow {
    /// What kind of transfer this is.
    pub kind: FlowKind,
    /// Where control goes, when the instruction alone determines it.
    ///
    /// Always `None` for an indirect transfer, and for a return, since both
    /// depend on a register's value at run time.
    pub target: Option<u32>,
    /// Whether the following instruction is reachable from this one.
    ///
    /// True for a conditional transfer, which may not be taken, and for a call,
    /// which is expected to come back to it.
    pub falls_through: bool,
}

impl Flow {
    /// Execution simply continues.
    const CONTINUE: Self = Self {
        kind: FlowKind::Continue,
        target: None,
        falls_through: true,
    };

    /// Returns whether this instruction ends a basic block.
    #[must_use]
    pub const fn terminates_block(&self) -> bool {
        !matches!(self.kind, FlowKind::Continue)
    }

    /// Returns whether control may leave without the target being known.
    #[must_use]
    pub const fn is_unresolved(&self) -> bool {
        self.terminates_block() && self.target.is_none()
    }
}

/// Branch condition bits meaning the branch is taken regardless of anything.
///
/// The field's high bit says to ignore the condition register, and the bit two
/// places down says to ignore the count register. With both set nothing is
/// tested, so the branch always transfers.
const BRANCH_ALWAYS: u32 = 0b10100;

/// Returns whether a branch condition field means always taken.
const fn always_taken(bo: u32) -> bool {
    bo & BRANCH_ALWAYS == BRANCH_ALWAYS
}

/// Sign extends the low `bits` of a value, staying in two's complement.
///
/// The result is kept unsigned so that adding it to an address wraps the way
/// the hardware does, without a signed conversion in between.
const fn sign_extend(value: u32, bits: u32) -> u32 {
    let field = (1u32 << bits) - 1;
    let sign = 1u32 << (bits - 1);
    let masked = value & field;

    if masked & sign == 0 {
        masked
    } else {
        masked | !field
    }
}

/// Resolves a branch displacement against the address it branches from.
const fn resolve(address: u32, displacement: u32, absolute: bool) -> u32 {
    if absolute {
        displacement
    } else {
        address.wrapping_add(displacement)
    }
}

/// Classifies what an instruction at `address` does to control flow.
pub(crate) fn classify(instruction: Instruction, address: u32) -> Flow {
    let word = instruction.word();
    let link = instruction.link_bit();
    let absolute = instruction.absolute_bit();
    let unconditional = always_taken(instruction.branch_condition());

    match instruction.opcode() {
        // The unconditional branch. Its displacement spans 24 bits above the
        // two the architecture requires to be zero.
        Opcode::B => {
            let target = resolve(address, sign_extend(word & 0x03ff_fffc, 26), absolute);
            Flow {
                kind: if link {
                    FlowKind::Call
                } else {
                    FlowKind::Branch
                },
                target: Some(target),
                falls_through: link,
            }
        }

        // The conditional branch, whose displacement is 14 bits above the same
        // two zero bits.
        Opcode::Bc => {
            let target = resolve(address, sign_extend(word & 0x0000_fffc, 16), absolute);
            Flow {
                kind: if link {
                    FlowKind::Call
                } else {
                    FlowKind::Branch
                },
                target: Some(target),
                falls_through: link || !unconditional,
            }
        }

        // A branch to the link register is how a function returns, unless it
        // takes the link itself, which makes it an indirect call.
        Opcode::Bclr => Flow {
            kind: if link {
                FlowKind::Call
            } else {
                FlowKind::Return
            },
            target: None,
            falls_through: link || !unconditional,
        },

        // A branch to the count register is how a computed target is reached,
        // including a virtual call when it takes the link.
        Opcode::Bcctr => Flow {
            kind: if link {
                FlowKind::Call
            } else {
                FlowKind::Indirect
            },
            target: None,
            falls_through: link || !unconditional,
        },

        // Returning from an interrupt leaves for an address held in a register
        // pair, so nothing follows it.
        Opcode::Rfid | Opcode::Hrfid => Flow {
            kind: FlowKind::Return,
            target: None,
            falls_through: false,
        },

        _ => Flow::CONTINUE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an unconditional branch with the given displacement.
    const fn branch(displacement: u32, absolute: bool, link: bool) -> u32 {
        (18 << 26) | (displacement & 0x03ff_fffc) | ((absolute as u32) << 1) | (link as u32)
    }

    /// Builds a conditional branch with the given condition and displacement.
    const fn conditional(bo: u32, displacement: u32, absolute: bool, link: bool) -> u32 {
        (16 << 26)
            | (bo << 21)
            | (2 << 16)
            | (displacement & 0xfffc)
            | ((absolute as u32) << 1)
            | (link as u32)
    }

    /// Builds a branch through the link or count register.
    const fn register_branch(xo: u32, bo: u32, link: bool) -> u32 {
        (19 << 26) | (bo << 21) | (xo << 1) | (link as u32)
    }

    /// Branch condition meaning always taken.
    const ALWAYS: u32 = 20;
    /// Branch condition meaning taken when a condition register bit is set.
    const IF_TRUE: u32 = 12;

    fn flow_of(word: u32, address: u32) -> Flow {
        classify(Instruction::decode(word), address)
    }

    #[test]
    fn an_arithmetic_instruction_does_not_terminate_a_block() {
        // addi r3, r4, 16
        let flow = flow_of(0x3864_0010, 0x8200_0000);

        assert_eq!(flow.kind, FlowKind::Continue);
        assert!(!flow.terminates_block());
        assert!(flow.falls_through);
        assert_eq!(flow.target, None);
    }

    #[test]
    fn a_relative_branch_resolves_against_the_instruction_address() {
        let flow = flow_of(branch(0x20, false, false), 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Branch);
        assert_eq!(flow.target, Some(0x8200_1020));
        assert!(!flow.falls_through);
        assert!(flow.terminates_block());
    }

    #[test]
    fn a_backward_branch_resolves_to_a_lower_address() {
        let flow = flow_of(branch(0x03ff_fffc, false, false), 0x8200_1000);

        assert_eq!(flow.target, Some(0x8200_0ffc));
    }

    /// An absolute branch ignores where it was branching from.
    #[test]
    fn an_absolute_branch_ignores_the_instruction_address() {
        let from_low = flow_of(branch(0x40, true, false), 0x8200_1000);
        let from_high = flow_of(branch(0x40, true, false), 0x8300_0000);

        assert_eq!(from_low.target, Some(0x40));
        assert_eq!(from_high.target, Some(0x40));
    }

    #[test]
    fn a_linking_branch_is_a_call_that_comes_back() {
        let flow = flow_of(branch(0x20, false, true), 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Call);
        assert_eq!(flow.target, Some(0x8200_1020));
        assert!(
            flow.falls_through,
            "a call returns to the following instruction"
        );
    }

    #[test]
    fn an_unconditional_conditional_branch_does_not_fall_through() {
        let flow = flow_of(conditional(ALWAYS, 0x40, false, false), 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Branch);
        assert_eq!(flow.target, Some(0x8200_1040));
        assert!(!flow.falls_through);
    }

    #[test]
    fn a_conditional_branch_reports_both_paths() {
        let flow = flow_of(conditional(IF_TRUE, 0x40, false, false), 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Branch);
        assert_eq!(flow.target, Some(0x8200_1040), "the taken path");
        assert!(flow.falls_through, "the untaken path");
        assert!(flow.terminates_block());
    }

    /// The canonical return, and the encoding a compiler emits most often.
    #[test]
    fn a_branch_to_the_link_register_is_a_return() {
        let flow = flow_of(0x4e80_0020, 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Return);
        assert_eq!(flow.target, None);
        assert!(!flow.falls_through);
        assert!(flow.is_unresolved());
    }

    #[test]
    fn a_conditional_return_still_falls_through() {
        let flow = flow_of(register_branch(16, IF_TRUE, false), 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Return);
        assert!(flow.falls_through);
    }

    #[test]
    fn a_branch_to_the_count_register_is_indirect() {
        let flow = flow_of(0x4e80_0420, 0x8200_1000);

        assert_eq!(flow.kind, FlowKind::Indirect);
        assert_eq!(flow.target, None, "an indirect branch claims no target");
        assert!(!flow.falls_through);
        assert!(flow.is_unresolved());
    }

    /// Taking the link through a register is how a virtual call is made, and it
    /// is a call rather than a plain indirect branch because it comes back.
    #[test]
    fn taking_the_link_through_a_register_is_an_indirect_call() {
        for word in [0x4e80_0421, 0x4e80_0021] {
            let flow = flow_of(word, 0x8200_1000);

            assert_eq!(flow.kind, FlowKind::Call, "{word:#010x}");
            assert_eq!(flow.target, None, "{word:#010x}");
            assert!(flow.falls_through, "{word:#010x}");
        }
    }

    #[test]
    fn returning_from_an_interrupt_is_a_return() {
        for xo in [18u32, 274] {
            let flow = flow_of(register_branch(xo, 0, false), 0x8200_1000);

            assert_eq!(flow.kind, FlowKind::Return);
            assert!(!flow.falls_through);
        }
    }

    #[test]
    fn only_the_always_taken_condition_skips_the_fall_through() {
        assert!(always_taken(20));
        assert!(always_taken(ALWAYS));

        for bo in [0u32, 4, 8, IF_TRUE, 16] {
            assert!(!always_taken(bo), "{bo} should be conditional");
        }
    }

    #[test]
    fn sign_extension_stays_in_twos_complement() {
        assert_eq!(sign_extend(0, 16), 0);
        assert_eq!(sign_extend(0x7ffc, 16), 0x7ffc);
        assert_eq!(sign_extend(0x8000, 16), 0xffff_8000);
        assert_eq!(sign_extend(0xfffc, 16), 0xffff_fffc);
        assert_eq!(sign_extend(0x03ff_fffc, 26), 0xffff_fffc);
    }

    /// A branch near the bottom of the address space wraps rather than
    /// panicking, which is what the hardware does.
    #[test]
    fn resolving_a_target_wraps_instead_of_overflowing() {
        let flow = flow_of(branch(0x03ff_fffc, false, false), 0x0000_0000);

        assert_eq!(flow.target, Some(0xffff_fffc));
    }

    #[test]
    fn classification_never_panics_over_arbitrary_words() {
        let mut state = 0x1234_5678u32;
        for _ in 0..20_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let flow = flow_of(state, state.rotate_left(7));

            if flow.kind == FlowKind::Continue {
                assert!(flow.falls_through);
                assert_eq!(flow.target, None);
            }
        }
    }
}
