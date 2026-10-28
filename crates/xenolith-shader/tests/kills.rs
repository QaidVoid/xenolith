//! Checking that a pixel shader can throw its pixel away on each comparison
//! the hardware offers.
//!
//! The four lane half has four operations that throw the pixel away where any
//! lane of its first source compares with its second: the same, greater, at
//! least as great, or different. Only the first was said, so a title whose
//! shaders throw pixels away on greater had them refused whole, and everything
//! they drew was passed over. A match drew nothing at all.
//!
//! The shaders here are written from the fields the decoder reads rather than
//! copied from a title, so what they exercise is stated rather than hoped for.

use xenolith_shader::{Kind, translate};

/// The numbers the four lane half names the operations by, with the
/// comparison each must become.
const KILLS: [(u32, u32); 4] = [
    (24, FLOAT_ORDERED_EQUAL),
    (25, FLOAT_ORDERED_GREATER_THAN),
    (26, FLOAT_ORDERED_GREATER_THAN_EQUAL),
    (27, FLOAT_UNORDERED_NOT_EQUAL),
];

const FLOAT_ORDERED_EQUAL: u32 = 180;
const FLOAT_UNORDERED_NOT_EQUAL: u32 = 183;
const FLOAT_ORDERED_GREATER_THAN: u32 = 186;
const FLOAT_ORDERED_GREATER_THAN_EQUAL: u32 = 190;
const KILL: u32 = 252;

/// One arithmetic instruction reading its first two sources from registers.
fn instruction(operation: u32, vector_lanes: u32, exports: bool) -> [u32; 3] {
    let first = (u32::from(exports) << 15) | ((vector_lanes & 0xf) << 16);
    let third = (operation << 24) | (1 << 31) | (1 << 30);
    [first, 0, third]
}

/// The control flow that runs `count` instructions from the first and then ends.
fn flow(count: u64) -> [u32; 3] {
    let run = (1u64 << 44) | (count << 12);
    let end = 2u64 << 44;
    let low = u32::try_from(run & 0xffff_ffff).unwrap_or_default();
    let middle = u32::try_from(((run >> 32) & 0xffff) | ((end & 0xffff) << 16)).unwrap_or_default();
    let high = u32::try_from((end >> 16) & 0xffff_ffff).unwrap_or_default();
    [low, middle, high]
}

/// A shader that throws its pixel away on `operation` and exports what is left.
fn throwing_away(operation: u32) -> Vec<u32> {
    let mut words = flow(2).to_vec();
    words.extend(instruction(operation, 0, false));
    words.extend(instruction(2, 0xf, true));
    words
}

/// Whether a module holds an instruction with this opcode and word count.
fn holds(module: &[u32], opcode: u32, length: u32) -> bool {
    module.contains(&((length << 16) | opcode))
}

#[test]
fn each_comparison_throws_the_pixel_away() {
    for (operation, comparing) in KILLS {
        let said = translate(Kind::Pixel, &throwing_away(operation))
            .unwrap_or_else(|refused| panic!("operation {operation} was refused: {refused:?}"));
        assert!(
            holds(&said.words, comparing, 5),
            "operation {operation} does not compare with opcode {comparing}"
        );
        assert!(
            holds(&said.words, KILL, 1),
            "operation {operation} does not throw the pixel away"
        );
    }
}

#[test]
fn a_vertex_cannot_be_thrown_away() {
    for (operation, _) in KILLS {
        assert!(
            translate(Kind::Vertex, &throwing_away(operation)).is_err(),
            "operation {operation} was said for a vertex shader"
        );
    }
}
