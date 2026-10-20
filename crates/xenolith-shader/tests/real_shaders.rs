//! Reading shaders a title actually handed the hardware.
//!
//! The words below were taken from a retail title as it ran, out of the
//! commands it wrote, and are kept here because a reading that works on words
//! made up to suit it has proved nothing. No game data is in this file: these
//! are two short programs, and what they are worth is that nobody here chose
//! them.
//!
//! What makes the reading right is not that each field looks plausible on its
//! own. It is that the parts agree. Every stretch of arithmetic a shader says
//! to run begins where the last one finished, the stretches cover the shader
//! with nothing left over and nothing twice, and each shader stops on the one
//! operation that means stop. A layout read wrongly does not do that.

use xenolith_shader::{Flow, FlowOperation};

/// A vertex shader, twenty four words, eight groups of three.
const VERTEX: [u32; 24] = [
    0x0000_1003,
    0x0000_1200,
    0xc400_0000,
    0x0000_1004,
    0x0000_1200,
    0xc200_0000,
    0x0000_1005,
    0x1006_1200,
    0x2200_0000,
    0xc800_0000,
    0x0000_0000,
    0x0200_0000,
    0xc800_c000,
    0x0000_0000,
    0xc200_0000,
    0xc888_c03e,
    0x0000_0000,
    0xc201_0100,
    0xc800_0000,
    0x0000_0000,
    0x0200_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

/// A pixel shader, nine words, three groups of three.
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

/// Every instruction a run of words holds, in the order they are stored.
fn flows(words: &[u32]) -> Vec<Flow> {
    let mut all = Vec::new();
    let mut at = 0;
    while let Some((low, high)) = Flow::pair(words, at) {
        all.push(low);
        all.push(high);
        at += 3;
    }
    all
}

#[test]
fn both_shaders_stop_on_the_operation_that_means_stop() {
    for words in [VERTEX.as_slice(), PIXEL.as_slice()] {
        assert!(
            flows(words).iter().any(|flow| flow.ends_the_shader()),
            "a shader has to say where it stops"
        );
    }
}

#[test]
fn what_is_run_is_covered_once_and_in_order() {
    for words in [VERTEX.as_slice(), PIXEL.as_slice()] {
        let mut next = None;
        for flow in flows(words) {
            let runs = matches!(
                flow.operation(),
                FlowOperation::Run | FlowOperation::RunAndEnd
            );
            if !runs {
                continue;
            }

            let count = u16::from(flow.count());
            if let Some(expected) = next {
                assert_eq!(
                    flow.address(),
                    expected,
                    "a stretch has to begin where the last one finished"
                );
            }
            next = Some(flow.address() + count);

            if flow.ends_the_shader() {
                break;
            }
        }

        assert!(next.is_some(), "a shader has to run something");
    }
}

#[test]
fn nothing_past_the_end_claims_to_run() {
    /* What follows the operation that means stop is padding, and padding that
     * decoded as something to run would be a layout read wrongly. */
    for words in [VERTEX.as_slice(), PIXEL.as_slice()] {
        let all = flows(words);
        let Some(end) = all.iter().position(|flow| flow.ends_the_shader()) else {
            panic!("a shader has to say where it stops");
        };

        for flow in all.iter().skip(end + 1) {
            assert!(
                matches!(
                    flow.operation(),
                    FlowOperation::Nothing | FlowOperation::Reserve
                ),
                "padding decoded as {:?}",
                flow.operation()
            );
        }
    }
}
