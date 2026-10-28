//! The instructions that do arithmetic.
//!
//! One of these is ninety six bits, three words, and does two things at once:
//! something across four lanes and something in one. Either half may be told to
//! do nothing useful, so a shader that only wants one of them still carries
//! both.

use crate::word_at;

/// What the four lane half of an instruction does.
///
/// The numbering is the hardware's own, read out of the five bits that name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorOperation {
    /// Adds.
    Add,
    /// Multiplies.
    Multiply,
    /// Takes the larger.
    Max,
    /// Takes the smaller.
    Min,
    /// One where equal, nothing where not.
    SetEqual,
    /// One where greater, nothing where not.
    SetGreater,
    /// One where greater or equal, nothing where not.
    SetGreaterOrEqual,
    /// One where unequal, nothing where not.
    SetNotEqual,
    /// The part after the point.
    Fraction,
    /// Towards nothing.
    Truncate,
    /// Towards the lower.
    Floor,
    /// Multiplies and adds.
    MultiplyAdd,
    /// Chooses on equal.
    ChooseOnEqual,
    /// Chooses on greater or equal.
    ChooseOnGreaterOrEqual,
    /// Chooses on greater.
    ChooseOnGreater,
    /// Four lanes multiplied and summed.
    DotFour,
    /// Three lanes multiplied and summed.
    DotThree,
    /// Two lanes multiplied and summed, plus a third.
    DotTwoAdd,
    /// Works out which face of a cube a direction points at.
    Cube,
    /// The largest of four lanes.
    MaxOfFour,
    /// Sets the predicate on equal and pushes.
    PredicateEqualPush,
    /// Sets the predicate on unequal and pushes.
    PredicateNotEqualPush,
    /// Sets the predicate on greater and pushes.
    PredicateGreaterPush,
    /// Sets the predicate on greater or equal and pushes.
    PredicateGreaterOrEqualPush,
    /// Throws the pixel away on equal.
    KillOnEqual,
    /// Throws the pixel away on greater.
    KillOnGreater,
    /// Throws the pixel away on greater or equal.
    KillOnGreaterOrEqual,
    /// Throws the pixel away on unequal.
    KillOnNotEqual,
    /// The distance terms a fixed function light wants.
    Distance,
    /// Takes the larger and keeps it for addressing.
    MaxAndKeep,
    /// A number this crate has no reading of.
    Unknown(u8),
}

impl VectorOperation {
    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Add,
            1 => Self::Multiply,
            2 => Self::Max,
            3 => Self::Min,
            4 => Self::SetEqual,
            5 => Self::SetGreater,
            6 => Self::SetGreaterOrEqual,
            7 => Self::SetNotEqual,
            8 => Self::Fraction,
            9 => Self::Truncate,
            10 => Self::Floor,
            11 => Self::MultiplyAdd,
            12 => Self::ChooseOnEqual,
            13 => Self::ChooseOnGreaterOrEqual,
            14 => Self::ChooseOnGreater,
            15 => Self::DotFour,
            16 => Self::DotThree,
            17 => Self::DotTwoAdd,
            18 => Self::Cube,
            19 => Self::MaxOfFour,
            20 => Self::PredicateEqualPush,
            21 => Self::PredicateNotEqualPush,
            22 => Self::PredicateGreaterPush,
            23 => Self::PredicateGreaterOrEqualPush,
            24 => Self::KillOnEqual,
            25 => Self::KillOnGreater,
            26 => Self::KillOnGreaterOrEqual,
            27 => Self::KillOnNotEqual,
            28 => Self::Distance,
            29 => Self::MaxAndKeep,
            other => Self::Unknown(other),
        }
    }
}

/// What the single lane half of an instruction does.
///
/// The numbering is the hardware's own. One number in the range names nothing,
/// which is a hole in the hardware's own list rather than a gap in this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarOperation {
    /// Adds two lanes of one source.
    Add,
    /// Adds what the last one produced.
    AddToPrevious,
    /// Multiplies two lanes of one source.
    Multiply,
    /// Multiplies what the last one produced.
    MultiplyPrevious,
    /// Multiplies what the last one produced, the other way round.
    MultiplyPreviousAgain,
    /// Takes the larger.
    Max,
    /// Takes the smaller.
    Min,
    /// One where equal.
    SetEqual,
    /// One where greater.
    SetGreater,
    /// One where greater or equal.
    SetGreaterOrEqual,
    /// One where unequal.
    SetNotEqual,
    /// The part after the point.
    Fraction,
    /// Towards nothing.
    Truncate,
    /// Towards the lower.
    Floor,
    /// Two raised to it.
    Exponent,
    /// The logarithm, clamped.
    LogarithmClamped,
    /// The logarithm.
    Logarithm,
    /// One over it, clamped.
    ReciprocalClamped,
    /// One over it, roughly.
    ReciprocalRough,
    /// One over it.
    Reciprocal,
    /// One over its root, clamped.
    ReciprocalRootClamped,
    /// One over its root, roughly.
    ReciprocalRootRough,
    /// One over its root.
    ReciprocalRoot,
    /// Takes the larger and keeps it for addressing.
    MaxAndKeep,
    /// Takes the larger and keeps it for addressing, without rounding.
    MaxAndKeepUnrounded,
    /// Subtracts.
    Subtract,
    /// Subtracts what the last one produced.
    SubtractFromPrevious,
    /// Sets the predicate on equal.
    PredicateEqual,
    /// Sets the predicate on unequal.
    PredicateNotEqual,
    /// Sets the predicate on greater.
    PredicateGreater,
    /// Sets the predicate on greater or equal.
    PredicateGreaterOrEqual,
    /// Turns the predicate around.
    PredicateInvert,
    /// Takes the predicate back off the stack.
    PredicatePop,
    /// Clears the predicate.
    PredicateClear,
    /// Puts the predicate back as it was.
    PredicateRestore,
    /// Throws the pixel away on equal.
    KillOnEqual,
    /// Throws the pixel away on greater.
    KillOnGreater,
    /// Throws the pixel away on greater or equal.
    KillOnGreaterOrEqual,
    /// Throws the pixel away on unequal.
    KillOnNotEqual,
    /// Throws the pixel away on one.
    KillOnOne,
    /// The root.
    Root,
    /// Multiplies by the first constant.
    MultiplyByConstant0,
    /// Multiplies by the second constant.
    MultiplyByConstant1,
    /// Adds the first constant.
    AddConstant0,
    /// Adds the second constant.
    AddConstant1,
    /// Subtracts the first constant.
    SubtractConstant0,
    /// Subtracts the second constant.
    SubtractConstant1,
    /// The sine.
    Sine,
    /// The cosine.
    Cosine,
    /// Keeps what the last one produced.
    KeepPrevious,
    /// A number this crate has no reading of.
    Unknown(u8),
}

impl ScalarOperation {
    #[allow(clippy::too_many_lines)]
    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Add,
            1 => Self::AddToPrevious,
            2 => Self::Multiply,
            3 => Self::MultiplyPrevious,
            4 => Self::MultiplyPreviousAgain,
            5 => Self::Max,
            6 => Self::Min,
            7 => Self::SetEqual,
            8 => Self::SetGreater,
            9 => Self::SetGreaterOrEqual,
            10 => Self::SetNotEqual,
            11 => Self::Fraction,
            12 => Self::Truncate,
            13 => Self::Floor,
            14 => Self::Exponent,
            15 => Self::LogarithmClamped,
            16 => Self::Logarithm,
            17 => Self::ReciprocalClamped,
            18 => Self::ReciprocalRough,
            19 => Self::Reciprocal,
            20 => Self::ReciprocalRootClamped,
            21 => Self::ReciprocalRootRough,
            22 => Self::ReciprocalRoot,
            23 => Self::MaxAndKeep,
            24 => Self::MaxAndKeepUnrounded,
            25 => Self::Subtract,
            26 => Self::SubtractFromPrevious,
            27 => Self::PredicateEqual,
            28 => Self::PredicateNotEqual,
            29 => Self::PredicateGreater,
            30 => Self::PredicateGreaterOrEqual,
            31 => Self::PredicateInvert,
            32 => Self::PredicatePop,
            33 => Self::PredicateClear,
            34 => Self::PredicateRestore,
            35 => Self::KillOnEqual,
            36 => Self::KillOnGreater,
            37 => Self::KillOnGreaterOrEqual,
            38 => Self::KillOnNotEqual,
            39 => Self::KillOnOne,
            40 => Self::Root,
            42 => Self::MultiplyByConstant0,
            43 => Self::MultiplyByConstant1,
            44 => Self::AddConstant0,
            45 => Self::AddConstant1,
            46 => Self::SubtractConstant0,
            47 => Self::SubtractConstant1,
            48 => Self::Sine,
            49 => Self::Cosine,
            50 => Self::KeepPrevious,
            other => Self::Unknown(other),
        }
    }
}

/// One arithmetic instruction, kept as the three words it was read from so that
/// a field is taken when it is asked for rather than up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alu {
    first: u32,
    second: u32,
    third: u32,
}

impl Alu {
    /// The instruction three words hold, or nothing where there are not three
    /// words to read.
    #[must_use]
    pub fn at(words: &[u32], at: usize) -> Option<Self> {
        Some(Self {
            first: word_at(words, at)?,
            second: word_at(words, at + 1)?,
            third: word_at(words, at + 2)?,
        })
    }

    /// What the four lane half does.
    #[must_use]
    pub fn vector_operation(self) -> VectorOperation {
        VectorOperation::from_bits(((self.third >> 24) & 0x1f) as u8)
    }

    /// What the single lane half does.
    #[must_use]
    pub fn scalar_operation(self) -> ScalarOperation {
        ScalarOperation::from_bits(((self.first >> 26) & 0x3f) as u8)
    }

    /// Which register the four lane half writes.
    #[must_use]
    pub fn vector_destination(self) -> u8 {
        (self.first & 0x3f) as u8
    }

    /// Which register the single lane half writes.
    #[must_use]
    pub fn scalar_destination(self) -> u8 {
        ((self.first >> 8) & 0x3f) as u8
    }

    /// Which lanes the four lane half writes.
    #[must_use]
    pub fn vector_lanes(self) -> u8 {
        ((self.first >> 16) & 0xf) as u8
    }

    /// Which lanes the single lane half writes.
    #[must_use]
    pub fn scalar_lanes(self) -> u8 {
        ((self.first >> 20) & 0xf) as u8
    }

    /// Whether what this writes leaves the shader rather than staying in a
    /// register.
    #[must_use]
    pub fn exports(self) -> bool {
        (self.first >> 15) & 1 == 1
    }

    /// Whether what the four lane half works out is kept between nought and
    /// one before it is written.
    #[must_use]
    pub fn vector_is_kept_to_one(self) -> bool {
        (self.first >> 24) & 1 == 1
    }

    /// Whether what the single lane half works out is kept between nought and
    /// one before it is written.
    #[must_use]
    pub fn scalar_is_kept_to_one(self) -> bool {
        (self.first >> 25) & 1 == 1
    }

    /// Whether this runs only where the predicate holds.
    #[must_use]
    pub fn is_predicated(self) -> bool {
        (self.second >> 28) & 1 == 1
    }

    /// Which way round an instruction that runs on the predicate reads it.
    #[must_use]
    pub fn predicate_holds(self) -> bool {
        (self.second >> 27) & 1 == 1
    }

    /// Whether each of the three sources names a register a shader writes, as
    /// against one of the constants handed to it from outside.
    ///
    /// Most of what this title's shaders read is a constant, so telling the two
    /// apart is not a detail: reading them all as registers reads almost
    /// everything from the wrong place.
    #[must_use]
    pub fn sources_are_registers(self) -> [bool; 3] {
        [
            (self.third >> 31) & 1 == 1,
            (self.third >> 30) & 1 == 1,
            (self.third >> 29) & 1 == 1,
        ]
    }

    /// Which registers the three sources name.
    #[must_use]
    pub fn sources(self) -> [u8; 3] {
        [
            ((self.third >> 16) & 0xff) as u8,
            ((self.third >> 8) & 0xff) as u8,
            (self.third & 0xff) as u8,
        ]
    }

    /// Which register the single lane half reads where it reads a constant and
    /// a register together.
    ///
    /// The number is not a field of its own. The console builds it from the
    /// lowest bit of the operation, the bit that otherwise says whether the
    /// third source names a register, and four bits of the third source's lane
    /// movement, which is why those operations are the only ones whose register
    /// cannot be read straight out of the instruction.
    #[must_use]
    pub fn single_lane_register(self) -> u8 {
        let operation = ((self.first >> 26) & 1) as u8;
        let instead = ((self.third >> 29) & 1) as u8;
        let moved = ((self.second >> 2) & 0xf) as u8;
        operation | (instead << 1) | (moved << 2)
    }

    /// How the lanes of each source are moved about before it is read.
    ///
    /// Two bits for each lane, and each says how far along to reach rather than
    /// which lane to take, so a source read as it stands is nought. Reading one
    /// as it stands regardless is reading the right register and the wrong
    /// numbers out of it, which more than half of one title's instructions do.
    #[must_use]
    pub fn source_lanes(self) -> [u8; 3] {
        [
            ((self.second >> 16) & 0xff) as u8,
            ((self.second >> 8) & 0xff) as u8,
            (self.second & 0xff) as u8,
        ]
    }

    /// Whether each source is read as how far it is from nought, before any
    /// turning of its sign.
    ///
    /// A register says so in the highest bit of its number, and the constants
    /// all say so together in one bit of the first word. One title's shader
    /// rearranges the pixels of a scene with a sum that comes out right only
    /// where a difference is read this way, and read with its sign the scene
    /// went to the screen a column in four out of place.
    #[must_use]
    pub fn sources_are_absolute(self) -> [bool; 3] {
        let constants = self.constants_are_absolute();
        let mut absolute = [constants; 3];
        for ((one, register), source) in absolute
            .iter_mut()
            .zip(self.sources_are_registers())
            .zip(self.sources())
        {
            if register {
                *one = source & 0x80 != 0;
            }
        }
        absolute
    }

    /// Whether every constant the instruction reads is read as how far it is
    /// from nought.
    #[must_use]
    pub fn constants_are_absolute(self) -> bool {
        (self.first >> 7) & 1 == 1
    }

    /// Whether a source, where it names a constant, counts it on from the
    /// address a shader last kept rather than from the start.
    ///
    /// The instruction has two bits for this, not three: the first constant it
    /// reads takes one and any after it the other, so which bit a source goes
    /// by depends on whether a source before it was a constant. A third bit
    /// says whether the count is on from the kept address or from how many
    /// times a loop has gone round, and only the first is read here.
    #[must_use]
    pub fn constant_follows_the_address(self, source: usize) -> bool {
        if (self.second >> 29) & 1 == 0 {
            return false;
        }
        let [one, two, _] = self.sources_are_registers();
        let first_constant = match source {
            0 => true,
            1 => one,
            _ => one && two,
        };
        let bit = if first_constant { 31 } else { 30 };
        (self.second >> bit) & 1 == 1
    }

    /// Whether each source is read with its sign turned about.
    #[must_use]
    pub fn sources_are_negated(self) -> [bool; 3] {
        [
            (self.second >> 26) & 1 == 1,
            (self.second >> 25) & 1 == 1,
            (self.second >> 24) & 1 == 1,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{Alu, ScalarOperation, VectorOperation};

    #[test]
    fn a_register_says_it_is_read_without_sign_in_its_highest_bit() {
        let registers = (1u32 << 31) | (1 << 30);
        let alu = Alu::at(&[0, 0, registers | (0x81 << 16) | (0x01 << 8) | 0x05], 0);
        assert_eq!(
            alu.map(Alu::sources_are_absolute),
            Some([true, false, false])
        );
        let alu = Alu::at(
            &[1 << 7, 0, registers | (0x81 << 16) | (0x01 << 8) | 0x05],
            0,
        );
        assert_eq!(
            alu.map(Alu::sources_are_absolute),
            Some([true, false, true])
        );
    }

    #[test]
    fn a_constant_follows_the_address_by_the_bit_its_place_among_constants_names() {
        /* The first and third sources name constants and the second a
         * register, so the third is the second constant and goes by the bit
         * for constants after the first. */
        let second_register = 1u32 << 30;
        let later_follows = (1u32 << 29) | (1 << 30);
        let alu = Alu::at(&[0, later_follows, second_register], 0);
        assert_eq!(
            alu.map(|alu| alu.constant_follows_the_address(0)),
            Some(false)
        );
        assert_eq!(
            alu.map(|alu| alu.constant_follows_the_address(2)),
            Some(true)
        );

        /* With the first two registers the third is the first constant. */
        let first_follows = (1u32 << 29) | (1 << 31);
        let alu = Alu::at(&[0, first_follows, (1 << 31) | (1 << 30)], 0);
        assert_eq!(
            alu.map(|alu| alu.constant_follows_the_address(2)),
            Some(true)
        );

        /* Counted on from a loop rather than the address, which is not read. */
        let alu = Alu::at(&[0, 1 << 31, 0], 0);
        assert_eq!(
            alu.map(|alu| alu.constant_follows_the_address(0)),
            Some(false)
        );
    }

    #[test]
    fn fewer_than_three_words_read_nothing() {
        assert!(Alu::at(&[0, 0], 0).is_none());
        assert!(Alu::at(&[0, 0, 0], 1).is_none());
    }

    #[test]
    fn the_two_halves_are_named_in_different_words() {
        /* The four lane half is named five bits into the top byte of the third
         * word, the single lane half six bits into the top of the first. */
        let Some(instruction) = Alu::at(&[11 << 26, 0, 15 << 24], 0) else {
            panic!("three words were given");
        };
        assert_eq!(instruction.vector_operation(), VectorOperation::DotFour);
        assert_eq!(instruction.scalar_operation(), ScalarOperation::Fraction);
    }

    #[test]
    fn a_number_with_no_reading_says_so() {
        /* Forty one names nothing in the hardware's own list. */
        assert_eq!(ScalarOperation::from_bits(41), ScalarOperation::Unknown(41));
        assert_eq!(VectorOperation::from_bits(31), VectorOperation::Unknown(31));
    }

    #[test]
    fn where_it_writes_comes_from_the_first_word() {
        let Some(instruction) = Alu::at(&[0x0000_8005, 0, 0], 0) else {
            panic!("three words were given");
        };
        assert_eq!(instruction.vector_destination(), 5);
        assert!(instruction.exports());
    }
}
