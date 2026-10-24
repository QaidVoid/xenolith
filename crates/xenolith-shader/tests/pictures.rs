//! Reading a picture where a shader says to, and asking about one.
//!
//! A fetch from a picture names which of a register's lanes hold the place to
//! read at, and a title packs two places into one register and reads either.
//! A shader that softens a shadow's edge blends texels itself, asking which
//! level a place reads at, reading at a level it says and a little to one side,
//! and asking how far between texels a place lies. The shader below is one a
//! title handed over, with its fetches changed into each of those, and what is
//! checked is that the module does what each says.

use xenolith_shader::{Kind, translate};

/// A pixel shader a title handed over, reading three pictures by the first two
/// lanes of a register.
const READS_THREE_PICTURES: [u32; 30] = [
    0x0015_3002,
    0x0000_1200,
    0xc400_0000,
    0x0000_4005,
    0x0000_2200,
    0x0000_0000,
    0x1018_1001,
    0x1f1f_fff8,
    0x0000_4000,
    0x1028_1001,
    0x1f1f_ffc7,
    0x0000_4000,
    0x1008_1001,
    0x1f1f_fe3f,
    0x0000_4000,
    0xc807_0000,
    0x00be_6d00,
    0x8001_fe00,
    0xa880_0000,
    0x0000_0000,
    0xc200_00ff,
    0xc88d_c000,
    0x00b2_1a1b,
    0xab00_fe00,
    0xc802_8000,
    0x0012_c000,
    0x9000_ff00,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

/// How many instructions of a kind a module holds.
fn how_many(module: &[u32], operation: u32) -> usize {
    let mut found = 0;
    let mut at = 5;
    while let Some(&first) = module.get(at) {
        let length = (first >> 16) as usize;
        if length == 0 {
            break;
        }
        if first & 0xffff == operation {
            found += 1;
        }
        at += length;
    }
    found
}

/// The lanes each pair of lanes taken out of a register is taken from, in the
/// order the module takes them.
fn pairs_taken(module: &[u32]) -> Vec<[u32; 2]> {
    let mut pairs = Vec::new();
    let mut at = 5;
    while let Some(&first) = module.get(at) {
        let length = (first >> 16) as usize;
        if length == 0 {
            break;
        }
        if first & 0xffff == 79
            && length == 7
            && let Some(&[one, other]) = module.get(at + 5..at + 7)
        {
            pairs.push([one, other]);
        }
        at += length;
    }
    pairs
}

#[test]
fn a_picture_is_read_at_the_lanes_a_fetch_names() {
    let mut words = READS_THREE_PICTURES;
    /* The first fetch reads by the third and fourth lanes. */
    words[6] = (words[6] & 0x03ff_ffff) | ((2 | (3 << 2)) << 26);

    let said = translate(Kind::Pixel, &words)
        .unwrap_or_else(|why| panic!("the shader was refused: {why:?}"));
    let pairs = pairs_taken(&said.words);
    assert_eq!(
        pairs.iter().filter(|pair| **pair == [2, 3]).count(),
        1,
        "the picture was not read at the last two lanes: {pairs:?}"
    );
    assert_eq!(
        pairs.iter().filter(|pair| **pair == [0, 1]).count(),
        2,
        "the other two pictures were not read at the first two lanes: {pairs:?}"
    );
}

#[test]
fn a_shader_blending_texels_itself_is_answered() {
    let mut words = READS_THREE_PICTURES;
    /* Which level the first place reads at, kept as the level to read at. */
    words[6] = (words[6] & !0x1f) | 0x11;
    words[9] = (words[9] & !0x1f) | 0x18;
    /* The last read at that level, half a texel left and half a texel down. */
    words[13] |= 1 << 29;
    words[14] |= (0x1f << 16) | (1 << 21);

    let said = translate(Kind::Pixel, &words)
        .unwrap_or_else(|why| panic!("the shader was refused: {why:?}"));
    assert_eq!(
        how_many(&said.words, 105),
        1,
        "the level a place reads at was not asked"
    );
    assert_eq!(
        how_many(&said.words, 88),
        1,
        "the picture was not read at the level said"
    );
    assert_eq!(
        how_many(&said.words, 103),
        1,
        "the picture was not moved by its size"
    );
}

#[test]
fn how_far_between_texels_and_a_fetch_on_the_predicate_are_answered() {
    let mut words = READS_THREE_PICTURES;
    words[6] = (words[6] & !0x1f) | 0x13;
    let unpredicated = translate(Kind::Pixel, &words)
        .unwrap_or_else(|why| panic!("the shader was refused: {why:?}"));
    /* The second fetch runs only where the predicate holds. */
    words[10] |= 1 << 31;
    words[11] |= 1 << 31;

    let said = translate(Kind::Pixel, &words)
        .unwrap_or_else(|why| panic!("the shader was refused: {why:?}"));
    assert_eq!(
        how_many(&said.words, 103),
        1,
        "how far between texels was not worked out from the size"
    );
    assert_eq!(
        how_many(&said.words, 169),
        how_many(&unpredicated.words, 169) + 1,
        "the fetch on the predicate was not kept to where it runs"
    );
}
