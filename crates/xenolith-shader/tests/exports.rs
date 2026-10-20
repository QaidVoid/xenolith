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

use xenolith_shader::{ALU_WORDS, Alu, CARRIED, Flow, FlowOperation, POSITION, scan};

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
