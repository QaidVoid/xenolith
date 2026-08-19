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
    entry: Option<u32>,
    words: Vec<u32>,
}

impl ImageBuilder {
    /// Starts an image loading at `base`.
    pub(crate) fn new(base: u32) -> Self {
        Self {
            base,
            entry: None,
            words: Vec::new(),
        }
    }

    /// Names where execution begins.
    pub(crate) fn entry(mut self, address: u32) -> Self {
        self.entry = Some(address);
        self
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

        Image::new(self.base, bytes, sections).with_entry_point(self.entry)
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

    /// Stores a doubleword. The low two bits of the field are the opcode.
    pub(crate) const fn std(rs: u32, ra: u32, displacement: u32) -> u32 {
        (62 << 26) | (rs << 21) | (ra << 16) | (displacement & 0xfffc)
    }

    /// Loads a doubleword.
    pub(crate) const fn ld(rt: u32, ra: u32, displacement: u32) -> u32 {
        (58 << 26) | (rt << 21) | (ra << 16) | (displacement & 0xfffc)
    }

    /// Stores a double precision float.
    pub(crate) const fn stfd(frs: u32, ra: u32, displacement: u32) -> u32 {
        (54 << 26) | (frs << 21) | (ra << 16) | (displacement & 0xffff)
    }

    /// Stores a vector register, offset by a register.
    pub(crate) const fn stvx(vrs: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (vrs << 21) | (ra << 16) | (rb << 11) | (231 << 1)
    }

    /// Stores a vector register through the console's extension, which splits
    /// the register number so it can reach past the standard range.
    pub(crate) const fn stvx128(vd: u32, ra: u32, rb: u32) -> u32 {
        0x1000_01c3 | ((vd & 0x1f) << 21) | (ra << 16) | (rb << 11) | (((vd >> 5) & 3) << 2)
    }

    /// Reads the link register into a general purpose register.
    ///
    /// The special purpose register field is split into two halves stored in
    /// the opposite order to the one they read in, so the link register, which
    /// is number eight, sits entirely in the lower half.
    pub(crate) const fn mflr(rt: u32) -> u32 {
        (31 << 26) | (rt << 21) | (8 << 16) | (339 << 1)
    }

    /// Stores a word and updates the base register, which is how a frame is
    /// allocated when the base is the stack pointer and the offset is negative.
    pub(crate) const fn stwu(rs: u32, ra: u32, displacement: u32) -> u32 {
        (37 << 26) | (rs << 21) | (ra << 16) | (displacement & 0xffff)
    }

    /// Adds an immediate to the upper half of a register.
    pub(crate) const fn addis(rt: u32, ra: u32, imm: u32) -> u32 {
        (15 << 26) | (rt << 21) | (ra << 16) | (imm & 0xffff)
    }

    /// Compares a register against an immediate without sign, which is how a
    /// switch checks that its index is in range.
    pub(crate) const fn cmpli(ra: u32, imm: u32) -> u32 {
        (10 << 26) | (ra << 16) | (imm & 0xffff)
    }

    /// Loads a byte, offset by a register.
    pub(crate) const fn lbzx(rt: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (87 << 1)
    }

    /// Loads a word, offset by a register.
    pub(crate) const fn lwzx(rt: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (23 << 1)
    }

    /// Adds two registers.
    pub(crate) const fn add(rt: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26) | (rt << 21) | (ra << 16) | (rb << 11) | (266 << 1)
    }

    /// Rotates a word left and masks it, naming its destination where most
    /// other forms name a source.
    pub(crate) const fn rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32) -> u32 {
        (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1)
    }

    /// Shifts a register left, which is how an index is scaled to address a
    /// table of words.
    pub(crate) const fn slwi(ra: u32, rs: u32, places: u32) -> u32 {
        rlwinm(ra, rs, places, 0, 31 - places)
    }

    /// Writes a register into the count register, which a branch then reads.
    pub(crate) const fn mtctr(rs: u32) -> u32 {
        (31 << 26) | (rs << 21) | (9 << 16) | (467 << 1)
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

    /// The switch encoders decide what recovery reads, so a mistake in one would
    /// make a test agree with a bug rather than with the architecture.
    #[test]
    fn the_switch_encoders_produce_what_the_decoder_reads_back() {
        let addis = Instruction::decode(encode::addis(12, 0, 0x8200));
        assert_eq!(addis.opcode().mnemonic(), "addis");
        assert_eq!(
            (addis.rt(), addis.ra(), addis.displacement()),
            (12, 0, -0x7e00)
        );

        let compare = Instruction::decode(encode::cmpli(10, 3));
        assert_eq!(compare.opcode().mnemonic(), "cmpli");
        assert_eq!((compare.ra(), compare.immediate()), (10, 3));

        let byte = Instruction::decode(encode::lbzx(0, 12, 10));
        assert_eq!(byte.opcode().mnemonic(), "lbzx");
        assert_eq!((byte.rt(), byte.ra(), byte.rb()), (0, 12, 10));

        let word = Instruction::decode(encode::lwzx(0, 12, 11));
        assert_eq!(word.opcode().mnemonic(), "lwzx");
        assert_eq!((word.rt(), word.ra(), word.rb()), (0, 12, 11));

        let sum = Instruction::decode(encode::add(12, 12, 0));
        assert_eq!(sum.opcode().mnemonic(), "add");
        assert_eq!((sum.rt(), sum.ra(), sum.rb()), (12, 12, 0));

        // A shift left by two is a rotate left by two keeping bits zero to
        // twenty nine, which is how a multiply by four is written.
        let shift = Instruction::decode(encode::slwi(0, 11, 2));
        assert_eq!(shift.opcode().mnemonic(), "rlwinm");
        assert_eq!(shift.ra(), 0, "the destination is named in the ra field");
        assert_eq!(shift.rt(), 11, "the source is named where others name it");
        assert_eq!(
            (shift.shift_amount(), shift.mask_begin(), shift.mask_end()),
            (2, 0, 29)
        );

        let count = Instruction::decode(encode::mtctr(12));
        assert_eq!(count.opcode().mnemonic(), "mtspr");
        assert_eq!(count.rt(), 12);
        assert_eq!(count.spr(), 9, "the count register is number nine");
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
