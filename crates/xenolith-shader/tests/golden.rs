//! The module a shader a title handed over becomes, word for word.
//!
//! Nothing here can say whether the translation draws the right thing. What it
//! can say is that the words coming out are the words that came out before, so
//! a change to the translator that alters them is caught here rather than in a
//! title nobody is looking at.
//!
//! The fixture was emitted once and read through before being committed. It is
//! checked structurally as well as compared, because a module that matches a
//! copy of itself says nothing about whether either is a module.
//!
//! No title and no external tool is needed: the words are in the file.

use xenolith_shader::{Kind, translate};

/// A pixel shader, nine words, as a retail title handed it over.
const PIXEL: [u32; 9] = [
    0x0000_0000,
    0x1001_c400,
    0x2200_0000,
    0xc80f_8000,
    0x0000_0000,
    0xc200_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

/// The expected module, as words.
// The fixture is checked by the test below, and a word that is not a word is a
// failure rather than something to carry on from.
#[allow(clippy::panic)]
fn expected() -> Vec<u32> {
    include_str!("fixtures/pixel_shader.txt")
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .map(|word| {
            let digits = word.strip_prefix("0x").unwrap_or(word);
            u32::from_str_radix(digits, 16)
                .unwrap_or_else(|error| panic!("{word} is not a word: {error}"))
        })
        .collect()
}

/// The module this shader translates to now.
#[allow(clippy::expect_used)]
fn emitted() -> Vec<u32> {
    translate(Kind::Pixel, &PIXEL)
        .expect("a shader a title handed over should translate")
        .words
}

/// The translation is the module that was committed.
#[test]
fn golden() {
    let wanted = expected();
    let got = emitted();

    assert_eq!(
        got.len(),
        wanted.len(),
        "the module is {} words where it was {}",
        got.len(),
        wanted.len()
    );

    let differing: Vec<usize> = got
        .iter()
        .zip(&wanted)
        .enumerate()
        .filter(|(_, (got, wanted))| got != wanted)
        .map(|(at, _)| at)
        .collect();
    let first = differing.first().copied().unwrap_or(0);
    assert!(
        differing.is_empty(),
        "{} of {} words differ, first at {first}: emitted {:#010x}, committed {:#010x}",
        differing.len(),
        wanted.len(),
        got[first],
        wanted[first],
    );
}

/// The committed words are a module rather than a run of numbers.
///
/// Every instruction states how many words it takes, so the walk either lands
/// exactly on the end or it does not. A word count changed in one place would
/// leave the rest of the walk reading operands as instructions.
#[test]
fn the_committed_words_are_a_well_formed_module() {
    let words = expected();

    assert_eq!(words.first().copied(), Some(0x0723_0203), "no magic number");
    assert_eq!(words[1], 0x0001_0300, "not SPIR-V 1.3");
    let bound = words[3];
    assert!(bound > 0, "a bound of zero means no identifiers at all");

    let mut at = 5;
    let mut instructions = 0u32;
    while at < words.len() {
        let word = words[at];
        let count = usize::try_from(word >> 16).unwrap_or(0);
        assert!(
            count > 0,
            "instruction {instructions} at word {at} says it takes no words"
        );
        at += count;
        instructions += 1;
    }
    assert_eq!(at, words.len(), "the last instruction runs past the end");
    assert!(instructions > 1, "a module holds more than one instruction");
}
