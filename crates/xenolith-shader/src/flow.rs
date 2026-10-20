//! The instructions that say which of the others to run.

use crate::word_at;

/// What a control flow instruction does.
///
/// The numbering is the hardware's own, read out of the four bits that name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowOperation {
    /// Does nothing, which is what fills a run out to its length.
    Nothing,
    /// Runs a stretch of the arithmetic instructions.
    Run,
    /// Runs a stretch, and is the last thing the shader does.
    RunAndEnd,
    /// Runs a stretch where a condition holds.
    RunIf,
    /// Runs a stretch where a condition holds, and ends there.
    RunIfAndEnd,
    /// Runs a stretch where the predicate holds.
    RunIfPredicated,
    /// Runs a stretch where the predicate holds, and ends there.
    RunIfPredicatedAndEnd,
    /// Begins a loop.
    LoopBegins,
    /// Ends a loop.
    LoopEnds,
    /// Calls where a condition holds.
    CallIf,
    /// Returns from a call.
    Return,
    /// Jumps where a condition holds.
    JumpIf,
    /// Reserves room for what the shader exports.
    Reserve,
    /// Runs a stretch where the predicate holds, having cleared it first.
    RunIfPredicatedClean,
    /// Runs a stretch where the predicate holds, having cleared it first, and
    /// ends there.
    RunIfPredicatedCleanAndEnd,
    /// Says the fetching a vertex shader does is finished.
    FetchDone,
    /// A number this crate has no reading of.
    Unknown(u8),
}

impl FlowOperation {
    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Nothing,
            1 => Self::Run,
            2 => Self::RunAndEnd,
            3 => Self::RunIf,
            4 => Self::RunIfAndEnd,
            5 => Self::RunIfPredicated,
            6 => Self::RunIfPredicatedAndEnd,
            7 => Self::LoopBegins,
            8 => Self::LoopEnds,
            9 => Self::CallIf,
            10 => Self::Return,
            11 => Self::JumpIf,
            12 => Self::Reserve,
            13 => Self::RunIfPredicatedClean,
            14 => Self::RunIfPredicatedCleanAndEnd,
            15 => Self::FetchDone,
            other => Self::Unknown(other),
        }
    }
}

/// One control flow instruction, kept as the forty eight bits it was read from
/// so that a field is taken when it is asked for rather than up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flow(u64);

impl Flow {
    /// The two instructions three words hold, or nothing where there are not
    /// three words to read.
    ///
    /// They are read as a pair because that is how they are stored: forty eight
    /// bits each, sharing three words with no gap between them.
    #[must_use]
    pub fn pair(words: &[u32], at: usize) -> Option<(Self, Self)> {
        let first = u64::from(word_at(words, at)?);
        let second = u64::from(word_at(words, at + 1)?);
        let third = u64::from(word_at(words, at + 2)?);

        let low = first | ((second & 0xffff) << 32);
        let high = (second >> 16) | (third << 16);

        Some((Self(low), Self(high)))
    }

    /// What this instruction does.
    #[must_use]
    pub fn operation(self) -> FlowOperation {
        FlowOperation::from_bits(((self.0 >> 44) & 0xf) as u8)
    }

    /// Where in the run of arithmetic instructions this one points.
    #[must_use]
    pub fn address(self) -> u16 {
        (self.0 & 0xfff) as u16
    }

    /// How many arithmetic instructions this one covers.
    #[must_use]
    pub fn count(self) -> u8 {
        ((self.0 >> 12) & 0x7) as u8
    }

    /// Whether the address is where it says rather than somewhere relative.
    #[must_use]
    pub fn is_absolute(self) -> bool {
        (self.0 >> 43) & 1 == 1
    }

    /// Which of the instructions a stretch runs are fetches rather than
    /// arithmetic, two bits for each in turn, the lower of the two saying
    /// fetch.
    #[must_use]
    pub fn sequence(self) -> u16 {
        ((self.0 >> 16) & 0xfff) as u16
    }

    /// Whether the instruction at this place in the stretch fetches rather
    /// than computes.
    #[must_use]
    pub fn fetches_at(self, which: usize) -> bool {
        which < 6 && (self.sequence() >> (which * 2)) & 1 == 1
    }

    /// Which way round a stretch that runs on the predicate reads it.
    ///
    /// A stretch may run where the predicate holds or where it does not, and
    /// the bit saying which sits just below the one saying how the address is
    /// counted.
    #[must_use]
    pub fn predicate_holds(self) -> bool {
        (self.0 >> 42) & 1 == 1
    }

    /// Whether this instruction is the last thing the shader does.
    #[must_use]
    pub fn ends_the_shader(self) -> bool {
        matches!(
            self.operation(),
            FlowOperation::RunAndEnd
                | FlowOperation::RunIfAndEnd
                | FlowOperation::RunIfPredicatedAndEnd
                | FlowOperation::RunIfPredicatedCleanAndEnd
        )
    }

    /// Which other control flow instruction a jump goes to. A jump reaches
    /// thirteen bits' worth where a stretch reaches twelve, so it is read wider
    /// than [`Flow::address`].
    #[must_use]
    pub fn jump_target(self) -> u16 {
        (self.0 & 0x1fff) as u16
    }

    /// Whether a jump asks after the predicate rather than one of the boolean
    /// constants, the bit sitting just above the one saying a jump always goes.
    #[must_use]
    pub fn is_predicated(self) -> bool {
        (self.0 >> 14) & 1 == 1
    }

    /// Whether a jump goes whatever the answer, asking after nothing at all.
    #[must_use]
    pub fn goes_regardless(self) -> bool {
        (self.0 >> 13) & 1 == 1
    }

    /// Which of the counts a loop reads to know how many times to go round.
    #[must_use]
    pub fn loop_named(self) -> u8 {
        ((self.0 >> 16) & 0x1f) as u8
    }

    /// Which of the truths a jump asks after, where it asks after one of those
    /// rather than the predicate.
    #[must_use]
    pub fn bool_index(self) -> u8 {
        ((self.0 >> 34) & 0xff) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::{Flow, FlowOperation};

    /// A stretch of six instructions where the first and the third are marked
    /// as fetches, which is the pattern the two bits for each in turn make.
    #[test]
    fn a_stretch_says_which_of_its_instructions_fetch() {
        /* Operation ten is a stretch that runs and ends, the count of six sits
         * above the address, and the sequence begins at bit sixteen. */
        let low = 0x0000_0000_0000_6000u64 | (0b01_00_01u64 << 16) | (10u64 << 44);
        let flow = Flow(low);
        assert_eq!(flow.count(), 6);
        assert!(flow.fetches_at(0));
        assert!(!flow.fetches_at(1));
        assert!(flow.fetches_at(2));
        assert!(!flow.fetches_at(3));
    }

    #[test]
    fn a_stretch_with_nothing_marked_fetches_nowhere() {
        let flow = Flow(0x0000_0000_0000_3000u64 | (10u64 << 44));
        assert!((0..6).all(|which| !flow.fetches_at(which)));
    }

    #[test]
    fn three_words_hold_two_instructions() {
        let words = [0x0000_0001, 0x0000_0000, 0x0000_0000];
        let Some((first, second)) = Flow::pair(&words, 0) else {
            panic!("three words were given");
        };
        assert_eq!(first.address(), 1);
        assert_eq!(second.address(), 0);
    }

    #[test]
    fn fewer_than_three_words_read_nothing() {
        assert!(Flow::pair(&[0, 0], 0).is_none());
        assert!(Flow::pair(&[0, 0, 0], 1).is_none());
    }

    #[test]
    fn the_operation_comes_from_the_top_four_bits() {
        /* The four bits naming the operation sit at forty four into the pair,
         * which is twelve into the word holding the middle of it. */
        let Some((first, _)) = Flow::pair(&[0, 0x0000_2000, 0], 0) else {
            panic!("three words were given");
        };
        assert_eq!(first.operation(), FlowOperation::RunAndEnd);
        assert!(first.ends_the_shader());
    }

    #[test]
    fn the_second_of_a_pair_starts_halfway_through_the_middle_word() {
        let Some((_, second)) = Flow::pair(&[0, 0x0001_0000, 0], 0) else {
            panic!("three words were given");
        };
        assert_eq!(second.address(), 1);
    }

    #[test]
    fn a_number_with_no_reading_says_so() {
        assert_eq!(FlowOperation::from_bits(200), FlowOperation::Unknown(200));
    }
}
