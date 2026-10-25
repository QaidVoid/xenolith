//! The instructions that go and get a shader's data.
//!
//! A draw hands a shader a number and nothing else. Where the vertex actually
//! is comes from the shader fetching it, out of memory the title filled in
//! beforehand, at an address it works out from that number. So these are not a
//! detail of a shader: for a vertex shader they are where the geometry is, and
//! a translation without them puts out a module that runs and draws nothing.
//!
//! Which slots in a stretch hold one of these is said by the control flow
//! rather than by the instruction itself, so [`crate::Flow::fetches_at`] is
//! what decides, and this only reads what is there once that has said so.

use crate::word_at;

/// What a fetch instruction goes and gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchKind {
    /// A vertex, out of memory the title filled in.
    Vertex,
    /// A texture, which needs a sampler rather than an address.
    Texture,
    /// Which of a texture's levels the hardware would read at a place, which
    /// a shader asks for to choose one itself.
    Level,
    /// How far a place lies between the texels about it, across and down,
    /// which is what a shader blending them itself weighs them by.
    Weights,
    /// Says which level of a texture the fetches after it that ask read at.
    SetLevel,
    /// A number this crate has no reading of.
    Unknown(u8),
}

impl FetchKind {
    fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Vertex,
            1 => Self::Texture,
            17 => Self::Level,
            19 => Self::Weights,
            24 => Self::SetLevel,
            other => Self::Unknown(other),
        }
    }
}

/// How the numbers a fetch brings back are stored.
///
/// The numbering is the hardware's own. Only the ones a title was seen to ask
/// for are named; the rest are reported rather than guessed at, because reading
/// a format wrongly puts out geometry that is wrong quietly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// One ordinary float.
    OneFloat,
    /// Two ordinary floats.
    TwoFloats,
    /// Three ordinary floats.
    ThreeFloats,
    /// Four ordinary floats.
    FourFloats,
    /// Four bytes in one word.
    FourBytes,
    /// Three of ten bits and one of two, in one word.
    ThreeTensAndATwo,
    /// Two of eleven bits and one of ten, in one word.
    TwoElevensAndATen,
    /// One of ten bits and two of eleven, in one word.
    ATenAndTwoElevens,
    /// Two halfwords in one word.
    TwoHalfwords,
    /// Four halfwords in two words.
    FourHalfwords,
    /// Two sixteen bit floats in one word.
    TwoHalfFloats,
    /// Four sixteen bit floats in two words.
    FourHalfFloats,
    /// A number this crate has no reading of.
    Unknown(u8),
}

/// One number inside a packed format: how wide it is, how far up the word it
/// sits, and which word holds it.
pub type Packed = (u32, u32, u32);

impl Format {
    fn from_bits(bits: u8) -> Self {
        match bits {
            6 => Self::FourBytes,
            7 => Self::ThreeTensAndATwo,
            16 => Self::TwoElevensAndATen,
            17 => Self::ATenAndTwoElevens,
            25 => Self::TwoHalfwords,
            26 => Self::FourHalfwords,
            31 => Self::TwoHalfFloats,
            32 => Self::FourHalfFloats,
            36 => Self::OneFloat,
            37 => Self::TwoFloats,
            57 => Self::ThreeFloats,
            38 => Self::FourFloats,
            other => Self::Unknown(other),
        }
    }

    /// How many numbers this format holds, or nothing where it has no reading.
    #[must_use]
    pub fn count(self) -> Option<u32> {
        match self {
            Self::OneFloat => Some(1),
            Self::TwoFloats | Self::TwoHalfwords | Self::TwoHalfFloats => Some(2),
            Self::ThreeFloats | Self::TwoElevensAndATen | Self::ATenAndTwoElevens => Some(3),
            Self::FourFloats
            | Self::FourBytes
            | Self::ThreeTensAndATwo
            | Self::FourHalfwords
            | Self::FourHalfFloats => Some(4),
            Self::Unknown(_) => None,
        }
    }

    /// Where each number sits where this format packs several into a word, or
    /// nothing where each number has a word to itself.
    ///
    /// The first number is the one lowest up the word, which is what settles
    /// which end a packed format is read from.
    #[must_use]
    pub fn packed(self) -> Option<Vec<Packed>> {
        Some(match self {
            Self::FourBytes => vec![(8, 0, 0), (8, 8, 0), (8, 16, 0), (8, 24, 0)],
            Self::ThreeTensAndATwo => vec![(10, 0, 0), (10, 10, 0), (10, 20, 0), (2, 30, 0)],
            Self::TwoElevensAndATen => vec![(11, 0, 0), (11, 11, 0), (10, 22, 0)],
            Self::ATenAndTwoElevens => vec![(10, 0, 0), (11, 10, 0), (11, 21, 0)],
            Self::TwoHalfwords => vec![(16, 0, 0), (16, 16, 0)],
            Self::FourHalfwords => vec![(16, 0, 0), (16, 16, 0), (16, 0, 1), (16, 16, 1)],
            _ => return None,
        })
    }

    /// How many words this format reads.
    #[must_use]
    pub fn words(self) -> Option<u32> {
        match self {
            Self::TwoHalfFloats => return Some(1),
            Self::FourHalfFloats => return Some(2),
            _ => {}
        }
        match self.packed() {
            Some(packed) => Some(
                packed
                    .iter()
                    .map(|(_, _, word)| word + 1)
                    .max()
                    .unwrap_or(1),
            ),
            None => self.count(),
        }
    }
}

/// One fetch instruction, kept as the three words it was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fetch {
    first: u32,
    second: u32,
    third: u32,
}

impl Fetch {
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

    /// What this goes and gets.
    #[must_use]
    pub fn kind(self) -> FetchKind {
        FetchKind::from_bits((self.first & 0x1f) as u8)
    }

    /// The register holding the number this fetches by.
    #[must_use]
    pub fn source(self) -> u8 {
        ((self.first >> 5) & 0x3f) as u8
    }

    /// Which of the source register's four numbers is the one to fetch by.
    #[must_use]
    pub fn source_component(self) -> u8 {
        ((self.first >> 30) & 0x3) as u8
    }

    /// Which of the source register's four numbers each of the places a
    /// picture is read at comes from, two bits for each in turn.
    #[must_use]
    pub fn source_swizzle(self) -> u8 {
        ((self.first >> 26) & 0x3f) as u8
    }

    /// How far from the place it names a texture fetch reads, across and
    /// down, in halves of a texel.
    #[must_use]
    pub fn moved_by(self) -> (i32, i32) {
        let across = i32::from_ne_bytes((self.third << 11).to_ne_bytes()) >> 27;
        let down = i32::from_ne_bytes((self.third << 6).to_ne_bytes()) >> 27;
        (across, down)
    }

    /// Whether a texture fetch reads at the level a shader last said rather
    /// than one the hardware chooses.
    #[must_use]
    pub fn reads_at_the_level_said(self) -> bool {
        (self.second >> 29) & 1 == 1
    }

    /// Whether this runs only where the predicate is one way.
    #[must_use]
    pub fn is_predicated(self) -> bool {
        (self.second >> 31) & 1 == 1
    }

    /// Which way the predicate has to be for a predicated fetch to run.
    #[must_use]
    pub fn predicate_holds(self) -> bool {
        (self.third >> 31) & 1 == 1
    }

    /// The register what this brings back is put in.
    #[must_use]
    pub fn destination(self) -> u8 {
        ((self.first >> 12) & 0x3f) as u8
    }

    /// Where the four numbers put into the destination come from, two bits for
    /// each in turn.
    #[must_use]
    pub fn destination_swizzle(self) -> u16 {
        (self.second & 0xfff) as u16
    }

    /// Which of the title's fetch constants says what memory to read.
    #[must_use]
    pub fn constant(self) -> u8 {
        ((self.first >> 20) & 0x1f) as u8
    }

    /// Which of the three fetches a constant's slot holds is meant.
    #[must_use]
    pub fn selector(self) -> u8 {
        ((self.first >> 25) & 0x3) as u8
    }

    /// Which pair of words in the title's fetch constants says what memory to
    /// read.
    ///
    /// The constants are kept in slots of six words, and fetching a vertex
    /// takes two of them, so three of these share a slot and the one meant is
    /// picked out by the selector.
    #[must_use]
    pub fn where_in_the_constants(self) -> u32 {
        u32::from(self.constant()) * 3 + u32::from(self.selector())
    }

    /// How the numbers this brings back are stored.
    #[must_use]
    pub fn format(self) -> Format {
        Format::from_bits(((self.second >> 16) & 0x3f) as u8)
    }

    /// Whether this leaves its format and stride to the fetch before it on the
    /// same constant rather than saying them itself.
    #[must_use]
    pub fn follows_another(self) -> bool {
        (self.second >> 30) & 1 == 1
    }

    /// Whether the numbers this brings back count either way from nothing
    /// rather than up from it.
    #[must_use]
    pub fn is_signed(self) -> bool {
        (self.second >> 12) & 1 == 1
    }

    /// Whether the numbers this brings back are brought into the range a
    /// fraction runs over rather than left as whole numbers.
    #[must_use]
    pub fn is_a_fraction(self) -> bool {
        (self.second >> 13) & 1 == 0
    }

    /// Whether a signed fraction is read so that it reaches both ends evenly
    /// rather than so that nothing is one of the values it takes.
    #[must_use]
    pub fn has_no_zero(self) -> bool {
        (self.second >> 14) & 1 == 1
    }

    /// How far apart two vertices are, counted in words.
    #[must_use]
    pub fn stride(self) -> u32 {
        self.third & 0xff
    }

    /// How far into a vertex what this fetches sits, counted in words.
    #[must_use]
    pub fn offset(self) -> u32 {
        (self.third >> 8) & 0x7f_ffff
    }
}

#[cfg(test)]
mod tests {
    use super::{Fetch, FetchKind, Format};

    /// Three fetch instructions a real title holds, as they were read out of
    /// its own shaders.
    const FROM_A_TITLE: [[u32; 3]; 3] = [
        [0x1df8_1000, 0x0025_3b08, 0x0000_0004],
        [0x3008_1000, 0x0039_3a88, 0x0000_0007],
        [0x5008_0001, 0x1f1f_f688, 0x0000_4000],
    ];

    #[test]
    fn two_sixteen_bit_floats_are_read_out_of_one_word() {
        let format = Format::from_bits(31);
        assert_eq!(format, Format::TwoHalfFloats);
        assert_eq!(format.count(), Some(2));
        assert_eq!(format.words(), Some(1));
    }

    #[test]
    fn a_fetch_a_title_holds_reads_as_one() {
        let Some(fetch) = Fetch::at(&FROM_A_TITLE[0], 0) else {
            panic!("three words were given");
        };

        assert_eq!(fetch.kind(), FetchKind::Vertex);
        assert_eq!(fetch.format(), Format::TwoFloats);
        assert_eq!(fetch.destination(), 1);
        assert_eq!(fetch.constant(), 31);
        assert_eq!(fetch.stride(), 4);
        assert_eq!(fetch.offset(), 0);
        assert!(!fetch.follows_another());
    }

    /// The title whose shaders these came from keeps live vertex constants in
    /// slots ninety four and ninety five, and this instruction names ninety
    /// five, which is what says the slot is worked out this way rather than
    /// some other.
    #[test]
    fn a_fetch_names_the_slot_a_title_actually_filled() {
        let Some(fetch) = Fetch::at(&FROM_A_TITLE[0], 0) else {
            panic!("three words were given");
        };

        assert_eq!(fetch.constant(), 31);
        assert_eq!(fetch.selector(), 2);
        assert_eq!(fetch.where_in_the_constants(), 95);
    }

    #[test]
    fn a_fetch_of_three_numbers_says_three() {
        let Some(fetch) = Fetch::at(&FROM_A_TITLE[1], 0) else {
            panic!("three words were given");
        };

        assert_eq!(fetch.format(), Format::ThreeFloats);
        assert_eq!(fetch.format().count(), Some(3));
        assert_eq!(fetch.stride(), 7);
    }

    #[test]
    fn a_fetch_of_a_texture_says_so_rather_than_reading_as_a_vertex() {
        let Some(fetch) = Fetch::at(&FROM_A_TITLE[2], 0) else {
            panic!("three words were given");
        };

        assert_eq!(fetch.kind(), FetchKind::Texture);
    }

    #[test]
    fn a_format_with_no_reading_says_so_rather_than_a_count() {
        assert_eq!(Format::from_bits(40), Format::Unknown(40));
        assert_eq!(Format::Unknown(40).count(), None);
        assert_eq!(Format::Unknown(40).packed(), None);
    }

    #[test]
    fn one_that_leaves_its_format_to_the_fetch_before_it_says_so() {
        let Some(fetch) = Fetch::at(&[0x0008_0000, 0x4025_3908, 0x0000_0200], 0) else {
            panic!("three words were given");
        };

        assert!(fetch.follows_another());
        assert_eq!(fetch.offset(), 2);
        assert_eq!(fetch.stride(), 0);
    }
}
