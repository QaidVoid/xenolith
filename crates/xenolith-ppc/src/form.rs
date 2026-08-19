//! Instruction encoding forms.
//!
//! Every PowerPC instruction is one 32-bit word whose first six bits are the
//! primary opcode. What the remaining bits mean depends on the form, and the
//! form also decides which bits identify the instruction as opposed to carrying
//! its operands. That second role is what [`Form::mask`] captures.
//!
//! Bit numbering in the architecture documentation counts from the most
//! significant bit. The shifts here are written in the host convention,
//! counting from the least significant bit, so a field documented as bits 6
//! through 10 is read as `(word >> 21) & 0x1f`.

/// The encoding form of an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Form {
    /// Primary opcode, a target register, a source register, and a 16-bit
    /// immediate or displacement.
    D,
    /// Primary opcode, three register fields, a 10-bit extended opcode, and the
    /// record bit.
    X,
    /// As [`Form::X`], but the extended opcode is nine bits and the bit above it
    /// enables overflow recording.
    XO,
}

impl Form {
    /// Returns the bits that identify an instruction of this form.
    ///
    /// Bits outside the mask carry operands, so two encodings that agree
    /// everywhere the mask covers are the same instruction.
    #[must_use]
    pub const fn mask(self) -> u32 {
        match self {
            Self::D => 0xfc00_0000,
            Self::X => 0xfc00_07fe,
            Self::XO => 0xfc00_03fe,
        }
    }

    /// Returns whether this form carries an extended opcode.
    #[must_use]
    pub const fn has_extended_opcode(self) -> bool {
        matches!(self, Self::X | Self::XO)
    }

    /// Returns the width in bits of this form's extended opcode.
    #[must_use]
    pub const fn extended_opcode_bits(self) -> u32 {
        match self {
            Self::D => 0,
            Self::XO => 9,
            Self::X => 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_cover_the_primary_opcode() {
        for form in [Form::D, Form::X, Form::XO] {
            assert_eq!(
                form.mask() & 0xfc00_0000,
                0xfc00_0000,
                "{form:?} does not identify the primary opcode"
            );
        }
    }

    /// The record bit selects a variant of the same instruction, so it must not
    /// be part of what identifies one.
    #[test]
    fn no_mask_covers_the_record_bit() {
        for form in [Form::D, Form::X, Form::XO] {
            assert_eq!(form.mask() & 1, 0, "{form:?} covers the record bit");
        }
    }

    /// The overflow enable bit selects a variant in the same way, so the
    /// narrower extended opcode of an XO-form instruction must leave it free.
    #[test]
    fn the_xo_form_mask_leaves_the_overflow_bit_free() {
        assert_eq!(Form::XO.mask() & (1 << 10), 0);
        assert_eq!(Form::X.mask() & (1 << 10), 1 << 10);
    }

    /// An X-form mask is strictly more specific than an XO-form mask, which is
    /// what lets the two share a primary opcode without ambiguity.
    #[test]
    fn the_x_form_mask_refines_the_xo_form_mask() {
        assert_eq!(Form::X.mask() & Form::XO.mask(), Form::XO.mask());
        assert_ne!(Form::X.mask(), Form::XO.mask());
    }

    #[test]
    fn extended_opcode_widths_match_the_masks() {
        for form in [Form::D, Form::X, Form::XO] {
            let bits = form.extended_opcode_bits();
            let covered = (form.mask() >> 1) & 0x3ff;
            assert_eq!(
                covered.count_ones(),
                bits,
                "{form:?} covers {covered:#x} but claims {bits} bits"
            );
        }
    }
}
