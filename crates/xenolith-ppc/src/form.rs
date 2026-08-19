//! Instruction encoding forms.
//!
//! Every PowerPC instruction is one 32-bit word whose first six bits are the
//! primary opcode. What the remaining bits mean depends on the form, and the
//! form also decides which bits identify the instruction as opposed to carrying
//! its operands. That second role is what [`Form::mask`] captures.
//!
//! Bit numbering in the architecture documentation counts from the most
//! significant bit. The positions here are written in the host convention,
//! counting from the least significant bit, so a field documented as bits 6
//! through 10 is read as `(word >> 21) & 0x1f`.
//!
//! A form is described by where its extended opcode sits and how wide it is.
//! The identifying mask follows from those two, so it cannot disagree with them.

/// The encoding form of an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Form {
    /// A target register, a source register, and a 16-bit immediate or
    /// displacement. Identified by its primary opcode alone.
    D,
    /// A source register, a target register, a shift amount, and a mask range,
    /// used by the 32-bit rotate instructions. Identified by primary opcode.
    M,
    /// As [`Form::D`], but the displacement gives up its low two bits to an
    /// extended opcode, which is how the doubleword accesses fit alongside each
    /// other under one primary opcode.
    DS,
    /// Three register fields, a 10-bit extended opcode, and the record bit.
    X,
    /// As [`Form::X`], but the extended opcode is nine bits and the bit above it
    /// enables overflow recording.
    XO,
    /// A source register, a target register, and a split shift amount, with a
    /// nine-bit extended opcode above the shift's high bit.
    XS,
    /// The 64-bit rotate instructions, with a three-bit extended opcode above
    /// the high bit of a split shift amount.
    MD,
    /// As [`Form::MD`], but the shift comes from a register, so the extended
    /// opcode is four bits.
    MDS,
}

/// Bits that identify the primary opcode.
const PRIMARY_MASK: u32 = 0xfc00_0000;

impl Form {
    /// Returns how far the extended opcode sits above the least significant bit.
    #[must_use]
    pub const fn extended_opcode_shift(self) -> u32 {
        match self {
            Self::D | Self::M | Self::DS => 0,
            Self::X | Self::XO | Self::MDS => 1,
            Self::XS | Self::MD => 2,
        }
    }

    /// Returns the extended opcode field once shifted down, or zero when the
    /// form has none.
    #[must_use]
    pub const fn extended_opcode_field(self) -> u32 {
        match self {
            Self::D | Self::M => 0,
            Self::DS => 0x3,
            Self::X => 0x3ff,
            Self::XO | Self::XS => 0x1ff,
            Self::MD => 0x7,
            Self::MDS => 0xf,
        }
    }

    /// Returns the bits that identify an instruction of this form.
    ///
    /// Bits outside the mask carry operands, so two encodings that agree
    /// everywhere the mask covers are the same instruction.
    #[must_use]
    pub const fn mask(self) -> u32 {
        PRIMARY_MASK | (self.extended_opcode_field() << self.extended_opcode_shift())
    }

    /// Returns whether this form has a record bit at the bottom of the word.
    ///
    /// A record bit selects a variant of one instruction rather than a
    /// different instruction, so a form that has one must leave it outside the
    /// identifying mask. [`Form::DS`] has none: its low two bits are the
    /// extended opcode itself, and [`Form::D`] spends them on its immediate.
    #[must_use]
    pub const fn has_record_bit(self) -> bool {
        matches!(
            self,
            Self::M | Self::X | Self::XO | Self::XS | Self::MD | Self::MDS
        )
    }

    /// Returns whether this form carries an extended opcode.
    #[must_use]
    pub const fn has_extended_opcode(self) -> bool {
        self.extended_opcode_field() != 0
    }

    /// Returns the width in bits of this form's extended opcode.
    #[must_use]
    pub const fn extended_opcode_bits(self) -> u32 {
        self.extended_opcode_field().count_ones()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every form this crate models.
    const ALL: &[Form] = &[
        Form::D,
        Form::M,
        Form::DS,
        Form::X,
        Form::XO,
        Form::XS,
        Form::MD,
        Form::MDS,
    ];

    #[test]
    fn masks_cover_the_primary_opcode() {
        for form in ALL {
            assert_eq!(
                form.mask() & PRIMARY_MASK,
                PRIMARY_MASK,
                "{form:?} does not identify the primary opcode"
            );
        }
    }

    /// The record bit selects a variant of the same instruction, so a form that
    /// has one must not let it identify the instruction.
    #[test]
    fn no_mask_covers_a_record_bit_the_form_actually_has() {
        for form in ALL {
            if !form.has_record_bit() {
                continue;
            }
            assert_eq!(form.mask() & 1, 0, "{form:?} covers its record bit");
        }
    }

    /// The forms without a record bit are the ones that spend the bottom of the
    /// word on something else, which is why the rule above has to be selective.
    #[test]
    fn only_the_forms_that_spend_the_low_bits_lack_a_record_bit() {
        assert!(!Form::D.has_record_bit());
        assert!(!Form::DS.has_record_bit());

        for form in ALL {
            if matches!(form, Form::D | Form::DS) {
                continue;
            }
            assert!(form.has_record_bit(), "{form:?} should have a record bit");
        }
    }

    /// The overflow enable bit selects a variant in the same way, so the
    /// narrower extended opcode of an XO-form instruction must leave it free.
    #[test]
    fn the_xo_form_mask_leaves_the_overflow_bit_free() {
        assert_eq!(Form::XO.mask() & (1 << 10), 0);
        assert_eq!(Form::X.mask() & (1 << 10), 1 << 10);
    }

    /// A form whose extended opcode is wider is strictly more specific, which
    /// is what lets forms share a primary opcode without ambiguity.
    #[test]
    fn wider_extended_opcodes_refine_narrower_ones() {
        for (wider, narrower) in [
            (Form::X, Form::XO),
            (Form::X, Form::XS),
            (Form::MDS, Form::MD),
        ] {
            assert_eq!(
                wider.mask() & narrower.mask(),
                narrower.mask(),
                "{wider:?} does not refine {narrower:?}"
            );
            assert_ne!(wider.mask(), narrower.mask());
        }
    }

    /// The mask is derived from the field position and width, so the three can
    /// never disagree. This pins the derivation itself.
    #[test]
    fn the_mask_follows_from_the_field_position_and_width() {
        for form in ALL {
            let field = form.extended_opcode_field();
            let shift = form.extended_opcode_shift();

            assert_eq!(form.mask(), PRIMARY_MASK | (field << shift));
            assert_eq!(form.extended_opcode_bits(), field.count_ones());
            assert_eq!(form.has_extended_opcode(), field != 0);
        }
    }

    /// An extended opcode field must be contiguous, or the shift and width
    /// would not describe it.
    #[test]
    fn extended_opcode_fields_are_contiguous() {
        for form in ALL {
            let field = form.extended_opcode_field();
            if field == 0 {
                continue;
            }
            assert_eq!(
                field.count_ones(),
                32 - field.leading_zeros(),
                "{form:?} has a field with a hole in it"
            );
        }
    }

    #[test]
    fn known_form_masks() {
        assert_eq!(Form::D.mask(), 0xfc00_0000);
        assert_eq!(Form::M.mask(), 0xfc00_0000);
        assert_eq!(Form::DS.mask(), 0xfc00_0003);
        assert_eq!(Form::X.mask(), 0xfc00_07fe);
        assert_eq!(Form::XO.mask(), 0xfc00_03fe);
        assert_eq!(Form::XS.mask(), 0xfc00_07fc);
        assert_eq!(Form::MD.mask(), 0xfc00_001c);
        assert_eq!(Form::MDS.mask(), 0xfc00_001e);
    }
}
