//! Building small images out of instruction sequences, for tests.
//!
//! Tests state the code they are about rather than pointing at a file, so a
//! failure names a sequence someone can read instead of an offset into a
//! binary. The encoders here cover only what the tests need, and each is
//! checked against the decoder rather than trusted.

use xenolith_xex::{Image, PageKind, Section};

/// Assembles instruction words into an image with one executable section.
pub(crate) struct ImageBuilder {
    base: u32,
    words: Vec<u32>,
}

impl ImageBuilder {
    /// Starts an image loading at `base`.
    pub(crate) fn new(base: u32) -> Self {
        Self {
            base,
            words: Vec::new(),
        }
    }

    /// Appends instruction words.
    pub(crate) fn code(mut self, words: &[u32]) -> Self {
        self.words.extend_from_slice(words);
        self
    }

    /// Builds the image.
    pub(crate) fn build(self) -> Image {
        let mut bytes = Vec::with_capacity(self.words.len() * 4);
        for word in &self.words {
            bytes.extend_from_slice(&word.to_be_bytes());
        }

        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let sections = vec![Section {
            start: self.base,
            size,
            kind: PageKind::Code,
        }];

        Image::new(self.base, bytes, sections)
    }
}

/// Encoders for the instructions the tests use.
pub(crate) mod encode {
    /// Adds an immediate to a register.
    pub(crate) const fn addi(rt: u32, ra: u32, imm: u32) -> u32 {
        (14 << 26) | (rt << 21) | (ra << 16) | (imm & 0xffff)
    }

    /// Branches by a relative displacement.
    pub(crate) const fn b(displacement: u32) -> u32 {
        (18 << 26) | (displacement & 0x03ff_fffc)
    }

    /// Branches by a relative displacement, taking the link.
    pub(crate) const fn bl(displacement: u32) -> u32 {
        b(displacement) | 1
    }

    /// Branches conditionally by a relative displacement.
    pub(crate) const fn bc(bo: u32, bi: u32, displacement: u32) -> u32 {
        (16 << 26) | (bo << 21) | (bi << 16) | (displacement & 0xfffc)
    }

    /// Returns to the caller.
    pub(crate) const fn blr() -> u32 {
        0x4e80_0020
    }

    /// Branches through the count register.
    pub(crate) const fn bctr() -> u32 {
        0x4e80_0420
    }

    /// Returns the displacement that branches `bytes` backward.
    ///
    /// Displacements are signed, and writing the two's complement value out is
    /// clearer than casting a negative literal at every call site.
    pub(crate) const fn back(bytes: u32) -> u32 {
        0u32.wrapping_sub(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenolith_ppc::{FlowKind, Instruction};

    /// The encoders are only useful if the decoder agrees with them, so each is
    /// checked rather than assumed.
    #[test]
    fn the_encoders_produce_what_the_decoder_reads_back() {
        let addi = Instruction::decode(encode::addi(3, 4, 16));
        assert_eq!(addi.opcode().mnemonic(), "addi");
        assert_eq!((addi.rt(), addi.ra(), addi.displacement()), (3, 4, 16));

        let branch = Instruction::decode(encode::b(8)).flow(0x8200_0000);
        assert_eq!(branch.kind, FlowKind::Branch);
        assert_eq!(branch.target, Some(0x8200_0008));

        let call = Instruction::decode(encode::bl(8)).flow(0x8200_0000);
        assert_eq!(call.kind, FlowKind::Call);
        assert!(call.falls_through);

        let taken = Instruction::decode(encode::bc(12, 0, 8)).flow(0x8200_0000);
        assert_eq!(taken.target, Some(0x8200_0008));
        assert!(taken.falls_through, "a conditional branch may not be taken");

        assert_eq!(
            Instruction::decode(encode::blr()).flow(0).kind,
            FlowKind::Return
        );
        assert_eq!(
            Instruction::decode(encode::bctr()).flow(0).kind,
            FlowKind::Indirect
        );
    }

    #[test]
    fn a_negative_displacement_branches_backward() {
        let back = Instruction::decode(encode::bc(12, 0, encode::back(8)));

        assert_eq!(back.flow(0x8200_0010).target, Some(0x8200_0008));
    }

    #[test]
    fn the_built_image_reads_back_the_words_it_was_given() {
        let image = ImageBuilder::new(0x8200_0000)
            .code(&[encode::addi(3, 4, 1), encode::blr()])
            .build();

        assert_eq!(image.u32(0x8200_0000).unwrap(), encode::addi(3, 4, 1));
        assert_eq!(image.u32(0x8200_0004).unwrap(), encode::blr());
        assert!(image.u32(0x8200_0008).is_err(), "the section ends");
    }
}
