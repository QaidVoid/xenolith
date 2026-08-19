//! A decoded instruction and the accessors that read its operands.
//!
//! A decoded instruction is its operation and the original word, and nothing
//! else. Operands are extracted when they are asked for rather than up front,
//! which keeps the value eight bytes and copyable. A full title runs to millions
//! of instructions and every later stage decodes them, so that size is the
//! difference between an analyzer holding the whole image comfortably and not.

use crate::flow::{Flow, classify};
use crate::form::Form;
use crate::table::{Opcode, decode_opcode};

/// A decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Instruction {
    opcode: Opcode,
    word: u32,
}

const _: () = assert!(
    size_of::<Instruction>() <= 8,
    "a decoded instruction must stay small enough to hold millions of"
);

impl Instruction {
    /// Decodes a 32-bit instruction word.
    ///
    /// Always succeeds. A word encoding nothing this crate recognizes yields
    /// [`Opcode::Unknown`], with the word preserved so a caller can report or
    /// investigate it.
    ///
    /// ```
    /// use xenolith_ppc::{Instruction, Opcode};
    ///
    /// let add = Instruction::decode(0x7c62_1a14);
    /// assert_eq!(add.opcode(), Opcode::Add);
    /// assert_eq!((add.rt(), add.ra(), add.rb()), (3, 2, 3));
    /// ```
    #[must_use]
    pub const fn decode(word: u32) -> Self {
        Self {
            opcode: decode_opcode(word),
            word,
        }
    }

    /// Returns the operation this instruction encodes.
    #[must_use]
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the original instruction word.
    #[must_use]
    pub const fn word(self) -> u32 {
        self.word
    }

    /// Returns whether the word encodes nothing this crate recognizes.
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(self.opcode, Opcode::Unknown)
    }

    /// Returns the encoding form, if the operation has one.
    #[must_use]
    pub fn form(self) -> Option<Form> {
        self.opcode.form()
    }

    /// Returns the primary opcode field.
    #[must_use]
    pub const fn primary_opcode(self) -> u32 {
        self.word >> 26
    }

    /// Returns the extended opcode, if this instruction's form carries one.
    #[must_use]
    pub fn extended_opcode(self) -> Option<u32> {
        let form = self.form()?;
        if !form.has_extended_opcode() {
            return None;
        }
        let width = form.extended_opcode_bits();
        Some((self.word >> 1) & ((1 << width) - 1))
    }

    /// Returns the target register field.
    #[must_use]
    pub const fn rt(self) -> u8 {
        register(self.word >> 21)
    }

    /// Returns the source register field, which shares its position with
    /// [`Instruction::rt`].
    #[must_use]
    pub const fn rs(self) -> u8 {
        register(self.word >> 21)
    }

    /// Returns the first operand register field.
    #[must_use]
    pub const fn ra(self) -> u8 {
        register(self.word >> 16)
    }

    /// Returns the second operand register field.
    #[must_use]
    pub const fn rb(self) -> u8 {
        register(self.word >> 11)
    }

    /// Returns the 16-bit immediate field without sign extension.
    #[must_use]
    pub const fn immediate(self) -> u16 {
        low_half(self.word)
    }

    /// Returns how far a word rotate rotates left.
    #[must_use]
    pub fn shift_amount(self) -> u8 {
        ((self.word >> 11) & 0x1f) as u8
    }

    /// Returns which condition register fields a move to it writes.
    ///
    /// The most significant bit selects the first field, which is the order the
    /// fields themselves sit in.
    ///
    /// ```
    /// use xenolith_ppc::Instruction;
    ///
    /// // mtcrf 255, r11, which writes every field
    /// let instruction = Instruction::decode(0x7d6f_f120);
    /// assert_eq!(instruction.condition_mask(), 0xff);
    /// ```
    #[must_use]
    pub const fn condition_mask(self) -> u8 {
        ((self.word >> 12) & 0xff) as u8
    }

    /// Returns which floating point status fields a move to it writes.
    ///
    /// The field sits further up the word than the condition register's does,
    /// so the two cannot share an accessor however alike they read.
    #[must_use]
    pub const fn status_mask(self) -> u8 {
        ((self.word >> 17) & 0xff) as u8
    }

    /// Returns the first bit of the mask a word rotate keeps.
    ///
    /// Bits are numbered from the most significant, so a mask beginning at zero
    /// keeps the top of the result.
    #[must_use]
    pub fn mask_begin(self) -> u8 {
        ((self.word >> 6) & 0x1f) as u8
    }

    /// Returns the last bit of the mask a word rotate keeps.
    ///
    /// A rotate left by `n` whose mask runs from zero to `31 - n` is a shift
    /// left by `n`, because every bit the rotate wrapped around is discarded.
    ///
    /// ```
    /// use xenolith_ppc::Instruction;
    ///
    /// // rlwinm r0, r0, 2, 0, 29, which multiplies by four
    /// let instruction = Instruction::decode(0x5400_103a);
    /// assert_eq!(instruction.shift_amount(), 2);
    /// assert_eq!(instruction.mask_begin(), 0);
    /// assert_eq!(instruction.mask_end(), 29);
    /// ```
    #[must_use]
    pub fn mask_end(self) -> u8 {
        ((self.word >> 1) & 0x1f) as u8
    }

    /// Returns how far a doubleword rotate rotates left.
    ///
    /// The field is six bits wide but there are only five bits left where the
    /// word rotates keep theirs, so the sixth is stored on its own further down
    /// the instruction. Reading only the lower five would halve every rotate
    /// past thirty one.
    #[must_use]
    pub fn long_shift_amount(self) -> u8 {
        let low = ((self.word >> 11) & 0x1f) as u8;
        let high = ((self.word >> 1) & 1) as u8;
        low | (high << 5)
    }

    /// Returns the six bit mask bound a doubleword rotate uses.
    ///
    /// Which end of the mask this is depends on the instruction: the left
    /// shifting forms bound the start and the right shifting forms bound the
    /// end. The field is stored with its two halves in the opposite order to
    /// the one they read in.
    #[must_use]
    pub fn long_mask_bound(self) -> u8 {
        let low = ((self.word >> 6) & 0x1f) as u8;
        let high = ((self.word >> 5) & 1) as u8;
        low | (high << 5)
    }

    /// Returns the displacement field, sign extended.
    ///
    /// Displacements are signed in the architecture, so a load from a negative
    /// offset off a base register reads as negative rather than as a very large
    /// positive number.
    ///
    /// The doubleword accesses spend the low two bits of the field on their
    /// extended opcode, so those bits are masked away rather than read as part
    /// of the offset. Reading them would put every such access two bytes off,
    /// but only for the variants whose opcode bits are not already zero, which
    /// is exactly the kind of error that hides.
    ///
    /// ```
    /// use xenolith_ppc::Instruction;
    ///
    /// // stw r14, -152(r1)
    /// assert_eq!(Instruction::decode(0x91c1_ff68).displacement(), -152);
    /// ```
    #[must_use]
    pub fn displacement(self) -> i32 {
        let raw = low_half(self.word);
        let field = if self.form() == Some(Form::DS) {
            raw & 0xfffc
        } else {
            raw
        };
        sign_extend_16(field)
    }

    /// Returns whether the record bit is set.
    ///
    /// A set record bit means the instruction updates a condition register
    /// field as a side effect, which is what distinguishes `add.` from `add`.
    ///
    /// Only meaningful for forms that carry it, which is why the bit sits
    /// outside the mask that identifies such an instruction. Every other form
    /// spends that bit on something else, so a caller that does not check
    /// [`Form::has_record_bit`] will read the low bit of an immediate and
    /// conclude that loading a constant updates a condition field.
    #[must_use]
    pub const fn record_bit(self) -> bool {
        self.word & 1 != 0
    }

    /// Returns the destination vector register of an extension instruction.
    ///
    /// The console reaches 128 vector registers by scattering the register
    /// number across the word: five bits sit where a standard vector
    /// instruction keeps its whole register field, and the top two sit near the
    /// bottom of the word in bits the standard encoding does not use.
    #[must_use]
    pub const fn vector_d(self) -> u8 {
        let low = (self.word >> 21) & 0x1f;
        let high = (self.word >> 2) & 0x3;
        register_wide((high << 5) | low)
    }

    /// Returns the source vector register, which shares its position with
    /// [`Instruction::vector_d`].
    #[must_use]
    pub const fn vector_s(self) -> u8 {
        self.vector_d()
    }

    /// Returns the first source vector register of an extension instruction.
    ///
    /// This one is split three ways rather than two, with its top two bits in
    /// separate single-bit fields at opposite ends of the word.
    #[must_use]
    pub const fn vector_a(self) -> u8 {
        let low = (self.word >> 16) & 0x1f;
        let bit5 = (self.word >> 6) & 0x1;
        let bit6 = (self.word >> 10) & 0x1;
        register_wide((bit6 << 6) | (bit5 << 5) | low)
    }

    /// Returns the second source vector register of an extension instruction.
    #[must_use]
    pub const fn vector_b(self) -> u8 {
        let low = (self.word >> 11) & 0x1f;
        let high = self.word & 0x3;
        register_wide((high << 5) | low)
    }

    /// Returns the special purpose register a move to or from one names.
    ///
    /// The field is ten bits stored as two halves in the opposite order to the
    /// one they read in, which is why this is worth an accessor rather than
    /// being extracted where it is needed.
    #[must_use]
    pub const fn spr(self) -> u32 {
        let low = (self.word >> 16) & 0x1f;
        let high = (self.word >> 11) & 0x1f;
        (high << 5) | low
    }

    /// Returns the branch condition field.
    ///
    /// Shares its position with the target register field. Meaningful only for
    /// the branch instructions, where it says what to test before transferring.
    #[must_use]
    pub const fn branch_condition(self) -> u32 {
        (self.word >> 21) & 0x1f
    }

    /// Returns the condition register bit a branch tests.
    #[must_use]
    pub const fn branch_condition_bit(self) -> u32 {
        (self.word >> 16) & 0x1f
    }

    /// Returns whether the link bit is set.
    ///
    /// A set link bit records the following address, which is what makes a
    /// branch a call rather than a jump. Shares its position with the record
    /// bit, so it is only meaningful on a form that has one.
    #[must_use]
    pub const fn link_bit(self) -> bool {
        self.word & 1 != 0
    }

    /// Returns whether the absolute addressing bit is set.
    ///
    /// A set bit means the displacement is the target rather than an offset
    /// from here. Only the two branch forms with a displacement carry it, and
    /// the register branches spend the same position on their extended opcode.
    #[must_use]
    pub fn absolute_bit(self) -> bool {
        self.form().is_some_and(Form::has_absolute_bit) && self.word & 0b10 != 0
    }

    /// Returns what this instruction does to control flow at `address`.
    ///
    /// The address matters because a relative branch resolves against it.
    ///
    /// ```
    /// use xenolith_ppc::{FlowKind, Instruction};
    ///
    /// // blr, the canonical return
    /// let flow = Instruction::decode(0x4e80_0020).flow(0x8200_1000);
    /// assert_eq!(flow.kind, FlowKind::Return);
    /// assert!(!flow.falls_through);
    /// ```
    #[must_use]
    pub fn flow(self, address: u32) -> Flow {
        classify(self, address)
    }

    /// Pairs this instruction with its address for rendering as text.
    ///
    /// The address is needed because a relative branch only means something
    /// once resolved against where it branches from.
    ///
    /// ```
    /// use xenolith_ppc::Instruction;
    ///
    /// let branch = Instruction::decode(0x4800_0020);
    /// assert_eq!(branch.render(0x8200_1000).to_string(), "b 0x82001020");
    /// ```
    #[must_use]
    pub const fn render(self, address: u32) -> crate::text::Rendered {
        crate::text::Rendered::new(self, address)
    }

    /// Returns whether the overflow enable bit is set.
    ///
    /// Only meaningful for forms that carry it, which is why the bit sits
    /// outside the mask that identifies such an instruction.
    #[must_use]
    pub const fn overflow_enable(self) -> bool {
        self.word & (1 << 10) != 0
    }
}

/// Narrows a reconstructed vector register number, which is always in range.
const fn register_wide(number: u32) -> u8 {
    (number & 0x7f) as u8
}

/// Extracts a five-bit register number from a shifted word.
const fn register(shifted: u32) -> u8 {
    (shifted & 0x1f) as u8
}

/// Extracts the low sixteen bits of a word.
const fn low_half(word: u32) -> u16 {
    (word & 0xffff) as u16
}

/// Sign extends a sixteen-bit value to a full signed integer.
const fn sign_extend_16(value: u16) -> i32 {
    let widened = value as i32;
    if value & 0x8000 == 0 {
        widened
    } else {
        widened - 0x1_0000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decoded_instruction_is_eight_bytes() {
        assert_eq!(size_of::<Instruction>(), 8);
        assert!(size_of::<Opcode>() <= 2);
    }

    #[test]
    fn decoding_is_total_over_arbitrary_words() {
        for word in [0u32, 1, 0xffff_ffff, 0x4e80_0020, 0x6000_0000] {
            let instruction = Instruction::decode(word);
            assert_eq!(instruction.word(), word);
        }
    }

    /// Uses a primary opcode the architecture leaves unassigned, so the word
    /// stays unrecognized however many instruction families are declared.
    #[test]
    fn an_unrecognized_word_keeps_its_bits() {
        let instruction = Instruction::decode(0x17ff_ffff);

        assert_eq!(instruction.primary_opcode(), 5, "primary 5 is unassigned");
        assert!(instruction.is_unknown());
        assert_eq!(instruction.opcode(), Opcode::Unknown);
        assert_eq!(instruction.word(), 0x17ff_ffff);
        assert_eq!(instruction.form(), None);
        assert_eq!(instruction.extended_opcode(), None);
    }

    #[test]
    fn reads_register_operands_of_a_three_register_instruction() {
        // add r3, r2, r3
        let instruction = Instruction::decode(0x7c62_1a14);

        assert_eq!(instruction.opcode(), Opcode::Add);
        assert_eq!(instruction.rt(), 3);
        assert_eq!(instruction.ra(), 2);
        assert_eq!(instruction.rb(), 3);
    }

    #[test]
    fn register_fields_never_exceed_five_bits() {
        for word in [0u32, 0xffff_ffff, 0x7fff_ffff, 0x8000_0000] {
            let instruction = Instruction::decode(word);
            assert!(instruction.rt() < 32);
            assert!(instruction.ra() < 32);
            assert!(instruction.rb() < 32);
        }
    }

    #[test]
    fn sign_extends_a_negative_displacement() {
        // stw r14, -152(r1)
        let instruction = Instruction::decode(0x91c1_ff68);

        assert_eq!(instruction.opcode(), Opcode::Stw);
        assert_eq!(instruction.rs(), 14);
        assert_eq!(instruction.ra(), 1);
        assert_eq!(instruction.displacement(), -152);
        assert_eq!(instruction.immediate(), 0xff68);
    }

    /// The doubleword accesses keep their extended opcode in the low two bits
    /// of the displacement field, so a variant whose opcode bits are not zero
    /// would otherwise read two bytes off.
    #[test]
    fn a_doubleword_displacement_excludes_the_extended_opcode() {
        // std and stdu store the same offset, differing only in the opcode bits.
        let std = Instruction::decode(0xf9c1_ff68);
        let stdu = Instruction::decode(0xf9c1_ff69);

        assert_eq!(std.opcode(), Opcode::Std);
        assert_eq!(stdu.opcode(), Opcode::Stdu);
        assert_eq!(std.displacement(), -152);
        assert_eq!(stdu.displacement(), -152, "the opcode bits are not offset");
    }

    /// The masking must not reach instructions whose field is a full 16 bits.
    #[test]
    fn an_ordinary_displacement_keeps_its_low_bits() {
        assert_eq!(Instruction::decode(0x3864_0003).displacement(), 3);
        assert_eq!(Instruction::decode(0x8064_0002).displacement(), 2);
    }

    #[test]
    fn sign_extension_covers_the_whole_range() {
        assert_eq!(sign_extend_16(0), 0);
        assert_eq!(sign_extend_16(0x7fff), 32767);
        assert_eq!(sign_extend_16(0x8000), -32768);
        assert_eq!(sign_extend_16(0xffff), -1);
    }

    /// The link register is the special purpose register a prologue reads to
    /// find where it was called from, so this is the one that has to be right.
    #[test]
    fn reads_a_special_purpose_register_number() {
        // mflr r12, which the encoding stores as a move from register eight.
        let instruction = Instruction::decode(0x7d88_02a6);

        assert_eq!(instruction.opcode(), Opcode::Mfspr);
        assert_eq!(instruction.spr(), 8);
        assert_eq!(instruction.rt(), 12);
    }

    #[test]
    fn a_special_purpose_register_uses_both_halves_of_its_field() {
        // The halves are stored swapped, so a number above thirty one proves
        // the high half is read from the right place.
        let word = 0x7c00_02a6 | (9 << 16) | (8 << 11);

        assert_eq!(Instruction::decode(word).spr(), (8 << 5) | 9);
    }

    #[test]
    fn reads_the_record_bit() {
        // add r3, r2, r3 and its recording variant
        assert!(!Instruction::decode(0x7c62_1a14).record_bit());
        assert!(Instruction::decode(0x7c62_1a15).record_bit());
    }

    /// The record bit selects a variant, so it must not change which
    /// instruction was decoded.
    #[test]
    fn the_record_bit_does_not_change_the_opcode() {
        assert_eq!(
            Instruction::decode(0x7c62_1a14).opcode(),
            Instruction::decode(0x7c62_1a15).opcode()
        );
    }

    /// The same holds for the overflow enable bit on the forms that carry it.
    #[test]
    fn the_overflow_bit_does_not_change_the_opcode() {
        let plain = Instruction::decode(0x7c62_1a14);
        let overflowing = Instruction::decode(0x7c62_1a14 | (1 << 10));

        assert_eq!(plain.opcode(), overflowing.opcode());
        assert!(!plain.overflow_enable());
        assert!(overflowing.overflow_enable());
    }

    #[test]
    fn reports_the_extended_opcode_width_of_each_form() {
        assert_eq!(
            Instruction::decode(0x7c62_1a14).extended_opcode(),
            Some(266)
        );
        assert_eq!(Instruction::decode(0x7c62_1838).extended_opcode(), Some(28));
        assert_eq!(Instruction::decode(0x3862_0001).extended_opcode(), None);
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;

    /// A vector load taken from the console's own documentation, whose operands
    /// are known independently. It reaches register 64, which is past anything
    /// the standard vector encoding can name, so it pins the reconstruction
    /// rather than merely exercising it.
    #[test]
    fn reconstructs_a_register_beyond_the_standard_range() {
        let instruction = Instruction::decode(0x100b_60cb);

        assert_eq!(instruction.opcode(), Opcode::Lvx128);
        assert_eq!(instruction.vector_d(), 64);
        assert_eq!(instruction.ra(), 11);
        assert_eq!(instruction.rb(), 12);
    }

    #[test]
    fn the_low_bits_alone_would_give_the_wrong_register() {
        let instruction = Instruction::decode(0x100b_60cb);

        assert_eq!((instruction.word() >> 21) & 0x1f, 0, "low bits are zero");
        assert_ne!(instruction.vector_d(), 0, "the high bits were dropped");
    }

    #[test]
    fn every_reconstructed_register_stays_in_range() {
        let mut state = 0x9e37_79b9u32;
        for _ in 0..50_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let instruction = Instruction::decode(state);

            assert!(instruction.vector_d() < 128);
            assert!(instruction.vector_a() < 128);
            assert!(instruction.vector_b() < 128);
            assert_eq!(instruction.vector_s(), instruction.vector_d());
        }
    }

    /// Each of the three registers draws on different bits, so a word that
    /// moves one must not move the others.
    #[test]
    fn the_three_register_fields_are_independent() {
        // Every bit clear, so each field is moved only by the bits under test.
        let base = 0x1000_0000u32;
        let field = |word: u32| {
            let i = Instruction::decode(word);
            (i.vector_d(), i.vector_a(), i.vector_b())
        };

        assert_eq!(field(base), (0, 0, 0));

        assert_eq!(field(base | (0x1f << 21)).0, 31, "destination low bits");
        assert_eq!(field(base | (0x3 << 2)).0, 96, "destination high bits");

        assert_eq!(field(base | (0x1f << 16)).1, 31, "first source low bits");
        assert_eq!(field(base | (1 << 6)).1, 32, "first source bit five");
        assert_eq!(field(base | (1 << 10)).1, 64, "first source bit six");

        assert_eq!(field(base | (0x1f << 11)).2, 31, "second source low bits");
        assert_eq!(field(base | 0x3).2, 96, "second source high bits");

        // Moving one field must leave the other two alone.
        assert_eq!(field(base | (0x1f << 21)), (31, 0, 0));
        assert_eq!(field(base | (0x1f << 11)), (0, 0, 31));
    }
}
