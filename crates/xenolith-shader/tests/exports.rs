//! Checking that every export a title's shaders make has somewhere to go.
//!
//! A vertex shader names a number for each thing it puts out: one for where the
//! vertex went, others for what it carries on to whatever runs next, and, on
//! this console, others again for writing straight to memory. The translator
//! carries a fixed number of the middle kind and knows the one number the
//! position uses. Anything else it has nowhere to put, and it refuses the shader
//! rather than passing the export over.
//!
//! What this counts is what a real title actually asks for, so the refusal is
//! known to be unreachable on this image rather than merely hoped to be. The
//! count of what is carried comes from the crate, so a change there cannot leave
//! this checking a number the translator no longer uses.

use std::collections::BTreeMap;

use xenolith_shader::{
    ALU_WORDS, Alu, CARRIED, Flow, FlowOperation, Kind, POSITION, scan, translate,
};

/// The first of the numbers that name a write straight to memory.
///
/// A shader using one of these is doing work whose result goes somewhere this
/// has nothing for, so finding one here would mean the translator needs to grow
/// rather than that a title is unusual.
const FIRST_TO_MEMORY: u8 = 32;

/// The instructions the control flow says to run, mirroring the translator.
///
/// Reading the arithmetic straight through instead would walk over the control
/// flow at the front and over the padding at the back, and report exports that
/// are not there.
fn stretches(words: &[u32]) -> Option<Vec<usize>> {
    let mut running = Vec::new();
    let mut at = 0;
    let mut ended = false;

    while let Some((low, high)) = Flow::pair(words, at) {
        for flow in [low, high] {
            if ended {
                continue;
            }
            match flow.operation() {
                FlowOperation::Run | FlowOperation::RunAndEnd => {
                    let first = usize::from(flow.address());
                    for step in 0..usize::from(flow.count()) {
                        if !flow.fetches_at(step) {
                            running.push(first.checked_add(step)?);
                        }
                    }
                    if flow.ends_the_shader() {
                        ended = true;
                    }
                }
                FlowOperation::Nothing | FlowOperation::Reserve => {}
                _ => return None,
            }
        }
        at += ALU_WORDS;
        if ended {
            return Some(running);
        }
    }

    None
}

/// Returns the image a title was built from.
///
/// # Panics
///
/// Where no image is named, because this is the test that reads one and a test
/// may not report a pass it did not earn. `scripts/check-oracles` names it.
#[allow(clippy::panic)]
fn image() -> Vec<u32> {
    let path = std::env::var_os("XENOLITH_SHADER_IMAGE").map_or_else(
        || panic!("XENOLITH_SHADER_IMAGE is not set, so this check could not run"),
        std::path::PathBuf::from,
    );
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));
    bytes
        .chunks_exact(4)
        .filter_map(|four| four.try_into().ok())
        .map(u32::from_be_bytes)
        .collect()
}

#[test]
#[ignore = "needs XENOLITH_SHADER_IMAGE"]
fn every_export_a_title_makes_has_somewhere_to_go() {
    let words = image();

    let mut destinations: BTreeMap<u8, usize> = BTreeMap::new();
    for one in scan(&words) {
        let Some(run) = words.get(one.at..one.at.saturating_add(one.words)) else {
            continue;
        };
        let Some(steps) = stretches(run) else {
            continue;
        };
        for step in steps {
            let Some(alu) = Alu::at(run, step.saturating_mul(ALU_WORDS)) else {
                continue;
            };
            if alu.exports() {
                *destinations.entry(alu.vector_destination()).or_default() += 1;
            }
        }
    }

    assert!(
        !destinations.is_empty(),
        "an image holds shaders that export something"
    );

    let carried = u8::try_from(CARRIED).unwrap_or(u8::MAX);
    let homeless: Vec<String> = destinations
        .keys()
        .filter(|to| **to != POSITION && **to >= carried)
        .map(|to| {
            if *to >= FIRST_TO_MEMORY {
                format!("{to} (a write straight to memory)")
            } else {
                format!("{to}")
            }
        })
        .collect();

    assert!(
        homeless.is_empty(),
        "shaders in this image export to {}, which the translator carries nothing to",
        homeless.join(", ")
    );
}

/// A pixel shader whose middle instruction names one lane of its single lane
/// half and no lane of its four lane half.
///
/// The two write masks are separate fields of the instruction, so naming one
/// lane of one half and none of the other is ordinary rather than malformed.
/// What the single lane half worked out still has somewhere to go, and a
/// translator that stops at the empty four lane mask drops it: the register
/// keeps whatever it held, and everything reading it afterwards reads that.
///
/// The shader is written from the fields the decoder reads rather than copied
/// from a title, so what it exercises is stated rather than hoped for. The last
/// instruction exports, which is what makes the module long enough for the
/// difference to show.
#[test]
fn a_write_to_one_lane_alone_is_not_dropped() {
    let words = one_lane_only();
    let said = translate(Kind::Pixel, &words).expect("the words are a shader");

    /* Two instructions do something and one exports. Without the single lane
     * write the middle instruction comes to nothing at all, and the module is
     * one instruction shorter. */
    let shorter = translate(Kind::Pixel, &words_without_the_one_lane_write())
        .expect("the words are a shader");
    assert!(
        said.words.len() > shorter.words.len(),
        "the single lane write is not in the module: {} words against {} without it",
        said.words.len(),
        shorter.words.len()
    );
}

/// One arithmetic instruction, from the fields the decoder reads.
fn instruction(
    vector_destination: u32,
    scalar_destination: u32,
    vector_lanes: u32,
    scalar_lanes: u32,
    exports: bool,
) -> [u32; 3] {
    let first = (vector_destination & 0x3f)
        | ((scalar_destination & 0x3f) << 8)
        | (u32::from(exports) << 15)
        | ((vector_lanes & 0xf) << 16)
        | ((scalar_lanes & 0xf) << 20);
    /* Max, reading its first two sources from registers. */
    let third = (2 << 24) | (1 << 31) | (1 << 30);
    [first, 0, third]
}

/// The control flow that runs `count` instructions from the first and then ends.
///
/// Two forty eight bit instructions share three words with no gap, the lower
/// taking the first word and half of the second and the higher taking the rest.
/// The masks keep each piece inside the thirty two bits it goes into.
fn flow(count: u64) -> [u32; 3] {
    let run = (1u64 << 44) | (count << 12);
    let end = 2u64 << 44;
    let low = u32::try_from(run & 0xffff_ffff).unwrap_or_default();
    let middle = u32::try_from(((run >> 32) & 0xffff) | ((end & 0xffff) << 16)).unwrap_or_default();
    let high = u32::try_from((end >> 16) & 0xffff_ffff).unwrap_or_default();
    [low, middle, high]
}

/// The words above, as a run a translator takes.
fn one_lane_only() -> Vec<u32> {
    let mut words = flow(3).to_vec();
    words.extend(instruction(0, 0, 0xf, 0, false));
    words.extend(instruction(0, 1, 0x0, 0x1, false));
    words.extend(instruction(0, 0, 0xf, 0, true));
    words
}

/// The same words with the middle instruction naming no lane of either half.
///
/// Nothing about this instruction reaches a register, so whatever the module
/// holds for it is what the translator makes of an instruction that writes
/// nothing. Comparing against it isolates the single lane write from every other
/// difference the middle instruction could make.
fn words_without_the_one_lane_write() -> Vec<u32> {
    let mut words = flow(3).to_vec();
    words.extend(instruction(0, 0, 0xf, 0, false));
    words.extend(instruction(0, 1, 0x0, 0x0, false));
    words.extend(instruction(0, 0, 0xf, 0, true));
    words
}
