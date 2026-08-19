//! Asserts that analysis completes and stays consistent over arbitrary code.
//!
//! Analysis walks a worklist over whatever bytes sit in an executable section.
//! Real titles put data and padding there, and a deliberately hostile image can
//! put anything at all, so every shape has to terminate rather than only the
//! shapes a compiler emits. Termination matters as much as the absence of a
//! panic here, because a worklist over code that branches into itself is
//! exactly the thing that loops forever.
//!
//! The invariants checked are the ones that would let a wrong answer through
//! quietly: blocks that overlap, edges that leave the function without being
//! marked unresolved, and a claim of more code than the image holds.
//!
//! Reachability is the one that matters most now that a recovered jump table
//! decides which blocks a function holds. A block claimed because a table named
//! it, but with no edge leading to it, is a block the graph says nothing about,
//! and it is exactly the failure feeding tables back into discovery can cause.

#![no_main]

use libfuzzer_sys::fuzz_target;
use xenolith_analysis::{Edge, analyze, detect, recover};
use xenolith_xex::{Image, PageKind, Section};

/// The address an image is placed at, chosen to match where a title loads.
const BASE: u32 = 0x8200_0000;

/// Bounds the work one input can ask for, so a timeout means a real hang.
const MAX_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 || data.len() > MAX_BYTES {
        return;
    }

    let size = u32::try_from(data.len()).unwrap_or(u32::MAX) & !3;
    if size == 0 {
        return;
    }

    let sections = vec![Section {
        start: BASE,
        size,
        kind: PageKind::Code,
    }];
    // The first word decides where execution starts, which lets an input aim
    // the entry point into the middle of an instruction or past the end.
    let entry = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let image = Image::new(BASE, data.to_vec(), sections).with_entry_point(Some(entry));

    let program = analyze(&image, &[]);

    let words = u64::from(size) / 4;
    assert!(
        program.claimed_instructions() <= words,
        "claimed more instructions than the section holds"
    );

    for function in program.functions() {
        let mut blocks: Vec<(u32, u32)> = function
            .blocks
            .iter()
            .map(|block| (block.start, block.end))
            .collect();
        blocks.sort_unstable();

        for pair in blocks.windows(2) {
            let [(_, first_end), (second_start, _)] = pair else {
                continue;
            };
            assert!(
                first_end <= second_start,
                "blocks {first_end:#x} and {second_start:#x} overlap"
            );
        }

        for (start, end) in &blocks {
            assert!(end > start, "a block must hold at least one instruction");
        }

        // An edge either lands on a block of this function or says it could not
        // be resolved. Anything else is an edge into nowhere.
        for edges in function.edges().values() {
            for edge in edges {
                let Some(target) = edge.target() else {
                    assert!(matches!(edge, Edge::Unresolved), "an edge lost its target");
                    continue;
                };
                assert!(
                    function.blocks.iter().any(|block| block.start == target),
                    "edge to {target:#010x} leaves the function"
                );
            }
        }

        // Every block has to be reachable from the function's first block. A
        // block reached only through a recovered table is reachable through the
        // edges that table produced, or it should not have been claimed.
        assert!(
            function.unreachable_blocks().is_empty(),
            "a block was claimed without the edge that justifies it"
        );

        // A branch reported as resolved must name at least one target, or
        // reporting it resolved says more than is known.
        for targets in function.resolved.values() {
            assert!(!targets.is_empty(), "a resolved branch names nothing");
        }

        let tables = recover(&image, function);
        for table in tables.recovered() {
            assert!(!table.targets.is_empty(), "a recovered table has no targets");
            for target in &table.targets {
                assert!(
                    image
                        .section_at(*target)
                        .is_some_and(|section| section.kind.is_executable()),
                    "a recovered target left executable memory"
                );
            }
        }
    }

    let helpers = detect(&image);
    for helper in helpers.all() {
        assert!(helper.end > helper.start, "a helper must span something");
        assert!(
            helper.last_register >= helper.first_register,
            "a helper must cover a run of registers"
        );
    }
});
