//! Finding the register save and restore helpers.
//!
//! Compilers for this console do not emit a prologue that saves every register
//! a function needs. They emit a call to a shared helper, entered part way along
//! depending on how many registers the caller wants saved. A recompiler has to
//! know where those helpers are, because a call into the middle of one is not a
//! call to a function that starts there.
//!
//! The tool this project is an alternative to makes the user supply the eight
//! addresses by hand, one per register file per direction. They are derivable,
//! and deriving them is the point.
//!
//! Detection matches the structure rather than the bytes: a run of memory
//! accesses whose register number rises by one and whose displacement rises by
//! a constant stride, ending in a return. That holds across all four register
//! files even though their instruction sequences look nothing alike, and it does
//! not care which scratch register a particular compiler picked.

use xenolith_ppc::{FlowKind, Instruction, Opcode};
use xenolith_xex::Image;

use crate::block::INSTRUCTION_SIZE;

/// Fewest steps a run must have before it is considered a helper.
///
/// An ordinary prologue saves a handful of registers the same way, so a short
/// run proves nothing on its own. What separates a helper is that its run is
/// followed by a return rather than by the body of a function, and that check
/// does most of the work. This bound only keeps the search from considering
/// pairs of unrelated stores.
const MIN_STEPS: usize = 4;

/// The first register each helper covers.
///
/// The calling convention names where the callee saved registers begin, and a
/// shared helper has to start there because it exists to serve a caller wanting
/// any number of them. A function's own epilogue restores only the registers it
/// happened to use, so it starts wherever that function's needs began.
///
/// This is what separates the two. Without it a four instruction epilogue
/// restoring the last few registers before returning looks exactly like a short
/// helper, and one was found in real code during development.
const fn first_covered_register(kind: HelperKind) -> u8 {
    match kind {
        HelperKind::GeneralPurpose | HelperKind::FloatingPoint | HelperKind::Vector => 14,
        // The wide vector helper covers the range the standard encoding cannot
        // name, which begins where that encoding leaves off.
        HelperKind::VectorWide => 64,
    }
}

/// How far past the run a return may sit.
///
/// The general purpose restore helper reloads the link register from the stack
/// and moves it into place before returning, which puts two instructions
/// between the end of its run and its return.
const RETURN_TOLERANCE: u32 = 3;

/// Which register file a helper covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HelperKind {
    /// The general purpose registers.
    GeneralPurpose,
    /// The floating point registers.
    FloatingPoint,
    /// The vector registers reachable by the standard encoding.
    Vector,
    /// The vector registers past the standard range, reached through the
    /// console's own vector extension.
    VectorWide,
}

impl HelperKind {
    /// Every kind a complete set contains.
    pub const ALL: [Self; 4] = [
        Self::GeneralPurpose,
        Self::FloatingPoint,
        Self::Vector,
        Self::VectorWide,
    ];

    /// Returns a readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::GeneralPurpose => "general purpose",
            Self::FloatingPoint => "floating point",
            Self::Vector => "vector",
            Self::VectorWide => "vector wide",
        }
    }
}

/// Whether a helper writes registers to memory or reads them back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HelperDirection {
    /// Writes registers to memory.
    Save,
    /// Reads registers back from memory.
    Restore,
}

impl HelperDirection {
    /// Every direction a complete set contains.
    pub const ALL: [Self; 2] = [Self::Save, Self::Restore];

    /// Returns a readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Restore => "restore",
        }
    }
}

/// A detected save or restore helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Helper {
    /// Address of the first instruction of the run.
    pub start: u32,
    /// Address one past the last instruction of the run.
    pub end: u32,
    /// Which register file it covers.
    pub kind: HelperKind,
    /// Whether it saves or restores.
    pub direction: HelperDirection,
    /// Lowest register the run touches.
    pub first_register: u8,
    /// Highest register the run touches.
    pub last_register: u8,
}

impl Helper {
    /// Returns whether an address falls inside the helper's run.
    ///
    /// A caller needing fewer registers enters part way along, so a call
    /// landing here is a call to this helper rather than to a function that
    /// begins at that address.
    #[must_use]
    pub const fn contains(&self, address: u32) -> bool {
        address >= self.start && address < self.end
    }
}

/// One access in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Step {
    kind: HelperKind,
    direction: HelperDirection,
    register: u8,
    displacement: i32,
    /// How many instructions the step consumed.
    words: u32,
}

/// Matches a single instruction access at `address`.
fn single_step(instruction: Instruction) -> Option<Step> {
    let (kind, direction) = match instruction.opcode() {
        Opcode::Std => (HelperKind::GeneralPurpose, HelperDirection::Save),
        Opcode::Ld => (HelperKind::GeneralPurpose, HelperDirection::Restore),
        Opcode::Stfd => (HelperKind::FloatingPoint, HelperDirection::Save),
        Opcode::Lfd => (HelperKind::FloatingPoint, HelperDirection::Restore),
        _ => return None,
    };

    Some(Step {
        kind,
        direction,
        register: instruction.rt(),
        displacement: instruction.displacement(),
        words: 1,
    })
}

/// Matches an immediate load followed by an indexed vector access.
///
/// The vector accesses take their offset from a register rather than from an
/// immediate field, so each step is a pair: the offset is materialized first and
/// the access follows. Which register carries it is left to the code rather than
/// assumed, and only has to be the same in both halves of the pair.
fn paired_step(first: Instruction, second: Instruction) -> Option<Step> {
    if first.opcode() != Opcode::Addi || first.ra() != 0 {
        return None;
    }
    let offset_register = first.rt();

    let (kind, direction, register) = match second.opcode() {
        Opcode::Stvx => (HelperKind::Vector, HelperDirection::Save, second.rt()),
        Opcode::Lvx => (HelperKind::Vector, HelperDirection::Restore, second.rt()),
        Opcode::Stvx128 => (
            HelperKind::VectorWide,
            HelperDirection::Save,
            second.vector_d(),
        ),
        Opcode::Lvx128 => (
            HelperKind::VectorWide,
            HelperDirection::Restore,
            second.vector_d(),
        ),
        _ => return None,
    };

    if second.ra() != offset_register {
        return None;
    }

    Some(Step {
        kind,
        direction,
        register,
        displacement: first.displacement(),
        words: 2,
    })
}

/// Matches whatever kind of step begins at `address`.
fn step_at(image: &Image, address: u32) -> Option<Step> {
    let first = Instruction::decode(image.u32(address).ok()?);

    if let Some(step) = single_step(first) {
        return Some(step);
    }

    let second = Instruction::decode(image.u32(address.checked_add(INSTRUCTION_SIZE)?).ok()?);
    paired_step(first, second)
}

/// Returns whether a return sits within reach of `address`.
fn returns_soon(image: &Image, address: u32) -> bool {
    (0..RETURN_TOLERANCE).any(|offset| {
        let at = address.wrapping_add(offset * INSTRUCTION_SIZE);
        image
            .u32(at)
            .is_ok_and(|word| Instruction::decode(word).flow(at).kind == FlowKind::Return)
    })
}

/// Matches the longest helper run beginning at `address`.
fn run_at(image: &Image, address: u32) -> Option<Helper> {
    let first = step_at(image, address)?;

    let mut last = first;
    let mut stride = None;
    let mut steps = 1;
    let mut cursor = address.checked_add(first.words * INSTRUCTION_SIZE)?;

    while let Some(step) = step_at(image, cursor) {
        if step.kind != first.kind
            || step.direction != first.direction
            || step.register != last.register.wrapping_add(1)
        {
            break;
        }

        let gap = step.displacement - last.displacement;
        match stride {
            None if gap > 0 => stride = Some(gap),
            Some(expected) if gap == expected => {}
            _ => break,
        }

        let Some(next) = cursor.checked_add(step.words * INSTRUCTION_SIZE) else {
            break;
        };
        last = step;
        cursor = next;
        steps += 1;
    }

    if steps < MIN_STEPS
        || first.register != first_covered_register(first.kind)
        || !returns_soon(image, cursor)
    {
        return None;
    }

    Some(Helper {
        start: address,
        end: cursor,
        kind: first.kind,
        direction: first.direction,
        first_register: first.register,
        last_register: last.register,
    })
}

/// What detection found, and what it expected to find.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Helpers {
    found: Vec<Helper>,
}

impl Helpers {
    /// Returns every detected helper, in address order.
    #[must_use]
    pub fn all(&self) -> &[Helper] {
        &self.found
    }

    /// Returns the helper containing an address, if any.
    #[must_use]
    pub fn containing(&self, address: u32) -> Option<&Helper> {
        self.found.iter().find(|helper| helper.contains(address))
    }

    /// Returns the kinds and directions expected but not found.
    ///
    /// A helper that is present and undetected is the failure that matters, and
    /// it is invisible unless absence is reported rather than inferred from a
    /// list of what turned up.
    #[must_use]
    pub fn missing(&self) -> Vec<(HelperKind, HelperDirection)> {
        let mut missing = Vec::new();
        for kind in HelperKind::ALL {
            for direction in HelperDirection::ALL {
                let present = self
                    .found
                    .iter()
                    .any(|helper| helper.kind == kind && helper.direction == direction);
                if !present {
                    missing.push((kind, direction));
                }
            }
        }
        missing
    }

    /// Returns whether every expected helper was found.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing().is_empty()
    }
}

/// Finds the save and restore helpers in an image.
///
/// Scans every executable section in address order and takes the longest run at
/// each point, which is what makes the reported address the start of the run
/// rather than one of the many points a caller may enter it at.
#[must_use]
pub fn detect(image: &Image) -> Helpers {
    let mut found = Vec::new();

    for section in image.executable_sections() {
        let mut address = section.start;

        while u64::from(address) < section.end() {
            if let Some(helper) = run_at(image, address) {
                address = helper.end;
                found.push(helper);
                continue;
            }
            let Some(next) = address.checked_add(INSTRUCTION_SIZE) else {
                break;
            };
            address = next;
        }
    }

    Helpers { found }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ImageBuilder, encode};

    /// Builds a run of single instruction accesses stepping away from a base.
    ///
    /// Displacements are built with unsigned arithmetic so the encoded field is
    /// produced directly, rather than by casting a negative number at each step.
    fn run(
        op: fn(u32, u32, u32) -> u32,
        base: u32,
        first: u32,
        count: u32,
        stride: u32,
    ) -> Vec<u32> {
        (0..count)
            .map(|i| {
                let displacement = encode::back(152).wrapping_add(i.wrapping_mul(stride));
                op(first + i, base, displacement & 0xffff)
            })
            .collect()
    }

    /// Builds a run of immediate load and indexed access pairs.
    fn paired_run(op: fn(u32, u32, u32) -> u32, first: u32, count: u32, from: u32) -> Vec<u32> {
        let mut code = Vec::new();
        for i in 0..count {
            let offset = encode::back(from).wrapping_add(i.wrapping_mul(16));
            code.push(encode::addi(11, 0, offset & 0xffff));
            code.push(op(first + i, 11, 12));
        }
        code
    }

    #[test]
    fn finds_a_general_purpose_save_helper() {
        let mut code = run(encode::std, 1, 14, 18, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert_eq!(helpers.all().len(), 1);
        let helper = helpers.all()[0];
        assert_eq!(helper.kind, HelperKind::GeneralPurpose);
        assert_eq!(helper.direction, HelperDirection::Save);
        assert_eq!(helper.start, 0x8200_0000);
        assert_eq!((helper.first_register, helper.last_register), (14, 31));
    }

    #[test]
    fn tells_a_restore_helper_from_a_save_one() {
        let mut code = run(encode::ld, 1, 14, 18, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert_eq!(helpers.all()[0].direction, HelperDirection::Restore);
    }

    #[test]
    fn finds_a_floating_point_helper_addressed_through_another_register() {
        let mut code = run(encode::stfd, 12, 14, 18, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert_eq!(helpers.all()[0].kind, HelperKind::FloatingPoint);
    }

    #[test]
    fn finds_a_vector_helper_built_from_pairs() {
        let mut code = paired_run(encode::stvx, 14, 18, 288);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert_eq!(helpers.all().len(), 1);
        assert_eq!(helpers.all()[0].kind, HelperKind::Vector);
        assert_eq!(helpers.all()[0].first_register, 14);
    }

    /// The wide variant reaches registers the standard encoding cannot name, so
    /// a detector that read only the low bits would report the wrong ones.
    #[test]
    fn finds_a_wide_vector_helper_reaching_past_the_standard_range() {
        let mut code = paired_run(encode::stvx128, 64, 8, 1024);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert_eq!(helpers.all().len(), 1);
        assert_eq!(helpers.all()[0].kind, HelperKind::VectorWide);
        assert_eq!(helpers.all()[0].first_register, 64);
        assert_eq!(helpers.all()[0].last_register, 71);
    }

    /// What separates a helper from an ordinary prologue is that a helper
    /// returns rather than carrying on into a function body.
    #[test]
    fn a_run_that_does_not_return_is_not_a_helper() {
        let mut code = run(encode::std, 1, 14, 18, 8);
        code.extend_from_slice(&[encode::addi(3, 4, 1); 8]);
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        assert!(detect(&image).all().is_empty());
    }

    #[test]
    fn a_run_too_short_to_be_a_helper_is_ignored() {
        let mut code = run(encode::std, 1, 30, 2, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        assert!(detect(&image).all().is_empty());
    }

    /// A function restoring the registers it used and returning has the same
    /// shape as a helper. What it does not have is the whole callee saved range,
    /// because it only ever needed part of it. Real code contains these, and one
    /// was reported as a helper before this rule existed.
    #[test]
    fn a_function_epilogue_is_not_a_helper() {
        let mut code = run(encode::ld, 1, 28, 4, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        assert!(
            detect(&image).all().is_empty(),
            "a partial run is an epilogue, not a shared helper"
        );
    }

    /// The rule is about where the run starts, not how long it is, so a run
    /// beginning at the right register is still a helper.
    #[test]
    fn a_run_starting_at_the_first_saved_register_is_a_helper() {
        let mut code = run(encode::ld, 1, 14, 4, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        assert_eq!(detect(&image).all().len(), 1);
    }

    #[test]
    fn a_run_whose_registers_are_not_consecutive_is_not_a_helper() {
        let code = vec![
            encode::std(14, 1, 0xff68),
            encode::std(16, 1, 0xff70),
            encode::std(18, 1, 0xff78),
            encode::std(20, 1, 0xff80),
            encode::blr(),
        ];
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        assert!(detect(&image).all().is_empty());
    }

    #[test]
    fn a_run_whose_stride_changes_is_not_a_helper() {
        let code = vec![
            encode::std(14, 1, 0xff68),
            encode::std(15, 1, 0xff70),
            encode::std(16, 1, 0xff90),
            encode::std(17, 1, 0xff98),
            encode::blr(),
        ];
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        assert!(detect(&image).all().is_empty());
    }

    /// A caller needing fewer registers jumps into the middle of the run, so the
    /// reported address has to be where the run starts rather than where that
    /// caller enters.
    #[test]
    fn the_reported_address_is_the_start_of_the_run() {
        let mut code = run(encode::std, 1, 14, 18, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert_eq!(helpers.all().len(), 1, "no suffix is reported separately");
        assert_eq!(helpers.all()[0].start, 0x8200_0000);
    }

    #[test]
    fn a_call_into_the_middle_is_attributed_to_the_helper() {
        let mut code = run(encode::std, 1, 14, 18, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);

        assert!(helpers.containing(0x8200_0020).is_some());
        assert!(helpers.containing(0x8200_0000).is_some());
        assert!(helpers.containing(0x8300_0000).is_none());
    }

    #[test]
    fn a_missing_kind_is_named_rather_than_left_out() {
        let mut code = run(encode::std, 1, 14, 18, 8);
        code.push(encode::blr());
        let image = ImageBuilder::new(0x8200_0000).code(&code).build();

        let helpers = detect(&image);
        let missing = helpers.missing();

        assert!(!helpers.is_complete());
        assert_eq!(missing.len(), 7, "one of eight was found");
        assert!(missing.contains(&(HelperKind::GeneralPurpose, HelperDirection::Restore)));
        assert!(!missing.contains(&(HelperKind::GeneralPurpose, HelperDirection::Save)));
    }

    #[test]
    fn an_image_with_no_helpers_reports_them_all_missing() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::addi(3, 4, 1), encode::blr()])
            .build();

        assert_eq!(detect(&image).missing().len(), 8);
    }

    #[test]
    fn detection_terminates_over_arbitrary_words() {
        let mut state = 0x1234_5678u32;
        let words: Vec<u32> = (0..4096)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                state
            })
            .collect();
        let image = ImageBuilder::new(0x8200_0000).code(&words).build();

        let _ = detect(&image);
    }
}
