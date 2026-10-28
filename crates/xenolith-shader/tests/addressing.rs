//! Checking that a shader can keep an address and read constants counted on
//! from it.
//!
//! A character is drawn by hanging each vertex from a bone, and the bone's
//! place is a run of constants a vertex shader reads counted on from an address
//! it keeps out of the vertex. Neither half was said: keeping the address was
//! passed over and the constants were read from the start, so every vertex hung
//! from the same bone and hands and gloves flew away from the players.
//!
//! The shaders here are written from the fields the decoder reads rather than
//! copied from a title, so what they exercise is stated rather than hoped for.

use xenolith_shader::{Kind, translate};

const CONVERT_FLOAT_TO_SIGNED: u32 = 110;
const EXT_INST: u32 = 12;
const SIGNED_CLAMP: u32 = 45;

const MAX: u32 = 2;
const MAX_AND_KEEP: u32 = 29;
const SINGLE_LANE_MAX_AND_KEEP: u32 = 23;
const POSITION: u32 = 62;

/// Every source names a register.
const REGISTERS: u32 = (1 << 31) | (1 << 30) | (1 << 29);

/// The control flow that runs `count` instructions from the one after it and
/// then ends.
fn flow(count: u64) -> [u32; 3] {
    let run = (1u64 << 44) | (count << 12) | 1;
    let end = 2u64 << 44;
    let low = u32::try_from(run & 0xffff_ffff).unwrap_or_default();
    let middle = u32::try_from(((run >> 32) & 0xffff) | ((end & 0xffff) << 16)).unwrap_or_default();
    let high = u32::try_from((end >> 16) & 0xffff_ffff).unwrap_or_default();
    [low, middle, high]
}

/// Puts out the larger of two constants, both counted on from the address.
fn read_after_the_address() -> [u32; 3] {
    let first = (1 << 15) | (0xf << 16) | POSITION;
    let second = (1 << 31) | (1 << 30) | (1 << 29);
    let third = (MAX << 24) | (10 << 16) | (10 << 8);
    [first, second, third]
}

/// A vertex shader that keeps an address with `keeping` and then reads after it.
fn keeping_then_reading(keeping: [u32; 3]) -> Vec<u32> {
    let mut words = flow(2).to_vec();
    words.extend(keeping);
    words.extend(read_after_the_address());
    words
}

/// Whether a module holds an instruction with this opcode and word count.
fn holds(module: &[u32], opcode: u32, length: u32) -> bool {
    module.contains(&((length << 16) | opcode))
}

/// Whether a module clamps a whole number as a signed one.
fn clamps_a_whole_number(module: &[u32]) -> bool {
    module.windows(5).any(|five| {
        five.first() == Some(&((8 << 16) | EXT_INST)) && five.get(4) == Some(&SIGNED_CLAMP)
    })
}

#[test]
fn the_single_lane_half_keeps_an_address_that_is_read_after() {
    /* Writing no lane at all, the way a title keeps an address and nothing
     * else. */
    let keeping = [SINGLE_LANE_MAX_AND_KEEP << 26, 0, (MAX << 24) | REGISTERS];
    let said = translate(Kind::Vertex, &keeping_then_reading(keeping))
        .unwrap_or_else(|refused| panic!("the shader was refused: {refused:?}"));
    assert!(
        holds(&said.words, CONVERT_FLOAT_TO_SIGNED, 4),
        "the address was never kept"
    );
    assert!(
        clamps_a_whole_number(&said.words),
        "the constants were not read counted on from the address"
    );
}

#[test]
fn the_four_lane_half_keeps_an_address_too() {
    let keeping = [0xf << 16 | 1, 0, (MAX_AND_KEEP << 24) | REGISTERS];
    let said = translate(Kind::Vertex, &keeping_then_reading(keeping))
        .unwrap_or_else(|refused| panic!("the shader was refused: {refused:?}"));
    assert!(
        holds(&said.words, CONVERT_FLOAT_TO_SIGNED, 4),
        "the address was never kept"
    );
}
