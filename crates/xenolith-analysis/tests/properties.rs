//! Property tests over the graphs analysis builds.
//!
//! The unit tests cover sequences someone thought to write. These cover the
//! invariants that have to hold across whole ranges of input, which is where a
//! walk over adversarial code goes wrong: a block that overlaps another, an
//! edge that leaves the function it belongs to, or a worklist that never
//! settles.
//!
//! Termination is as much the subject here as correctness. Code that branches
//! into itself, into the middle of an instruction, or off the end of a section
//! is exactly the shape that makes a fixpoint loop forever, and none of it is
//! unusual in a real title.

use std::collections::BTreeSet;

use proptest::prelude::*;
use xenolith_analysis::{Edge, analyze, blocks_within, detect, recover};
use xenolith_xex::{Image, PageKind, Section};

/// The address images are placed at, matching where a title loads.
const BASE: u32 = 0x8200_0000;

/// Builds an image over `words`, all one executable section.
fn image_of(words: &[u32], entry: Option<u32>) -> Image {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
    let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let sections = vec![Section {
        start: BASE,
        size,
        kind: PageKind::Code,
    }];
    Image::new(BASE, bytes, sections).with_entry_point(entry)
}

/// Checks every graph invariant against one image, or panics naming the breach.
fn check(image: &Image, words: usize) {
    let program = analyze(image, &[]);

    assert!(
        program.claimed_instructions() <= words as u64,
        "claimed more instructions than the section holds"
    );

    for function in program.functions() {
        let mut spans: Vec<(u32, u32)> = function
            .blocks
            .iter()
            .map(|block| (block.start, block.end))
            .collect();
        spans.sort_unstable();

        for (start, end) in &spans {
            assert!(end > start, "a block must hold at least one instruction");
        }
        for pair in spans.windows(2) {
            let [(_, first_end), (second_start, _)] = pair else {
                continue;
            };
            assert!(
                first_end <= second_start,
                "blocks overlap at {first_end:#010x} and {second_start:#010x}"
            );
        }

        // Every block of a function is reachable from its entry, so the blocks
        // together are exactly the instructions the function claims.
        let covered: u64 = spans
            .iter()
            .map(|(start, end)| u64::from(end - start) / 4)
            .sum();
        assert_eq!(
            covered,
            u64::from(function.instruction_count()),
            "the blocks do not cover what the function claims"
        );

        // A block claimed because a table named it, with no edge leading to it,
        // is a block the graph says nothing about. That is the failure feeding
        // tables back into discovery can cause, so it is checked everywhere.
        assert!(
            function.unreachable_blocks().is_empty(),
            "a block was claimed without the edge that justifies it"
        );

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

        for table in recover(image, function).recovered() {
            assert!(
                !table.targets.is_empty(),
                "a recovered table has no targets"
            );
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

    for helper in detect(image).all() {
        assert!(helper.end > helper.start, "a helper must span something");
    }
}

proptest! {
    /// Analysis has to answer for arbitrary words, because a real title puts
    /// data and padding in its executable sections and analysis reads whatever
    /// is there.
    #[test]
    fn arbitrary_code_produces_a_consistent_graph(
        words in prop::collection::vec(any::<u32>(), 1..512),
    ) {
        let image = image_of(&words, Some(BASE));
        check(&image, words.len());
    }

    /// The entry point is not trusted to be sensible. Pointing it into the
    /// middle of an instruction or past the end of the section must be answered
    /// rather than assumed away.
    #[test]
    fn an_entry_point_anywhere_is_answered(
        words in prop::collection::vec(any::<u32>(), 1..256),
        entry in any::<u32>(),
    ) {
        let image = image_of(&words, Some(entry));
        check(&image, words.len());
    }

    /// A branch to itself is the smallest loop there is, and a worklist that
    /// does not settle on it will not settle on anything.
    #[test]
    fn code_that_branches_to_itself_terminates(
        before in prop::collection::vec(any::<u32>(), 0..64),
        after in prop::collection::vec(any::<u32>(), 0..64),
    ) {
        let mut words = before;
        // A branch of displacement zero targets the instruction it is.
        words.push(0x4800_0000);
        words.extend(after);

        let image = image_of(&words, Some(BASE));
        check(&image, words.len());
    }

    /// Every branch reachable from the entry aims backward, so following them
    /// revisits blocks already walked.
    #[test]
    fn code_that_only_branches_backward_terminates(
        count in 1u32..96,
    ) {
        // Each word branches back four bytes, to the one before it.
        let words = vec![0x4bff_fffc; count as usize];

        let image = image_of(&words, Some(BASE + (count - 1) * 4));
        check(&image, words.len());
    }

    /// A branch past the end of the section resolves to nothing mapped, which
    /// has to be reported rather than walked into.
    #[test]
    fn a_branch_off_the_end_terminates(
        words in prop::collection::vec(any::<u32>(), 1..64),
    ) {
        let mut words = words;
        // The largest forward displacement a branch can encode.
        words.push(0x49ff_fffc);

        let image = image_of(&words, Some(BASE));
        check(&image, words.len());
    }

    /// A call whose target is another function must not pull that function's
    /// blocks into the caller, which is how a walk once claimed code twice.
    #[test]
    fn a_call_into_other_code_terminates(
        distance in 1u32..64,
    ) {
        let mut words = vec![0x6000_0000; 128];
        // A call forward, then a return, with the callee also returning.
        let target = (distance as usize) + 1;
        words[0] = 0x4800_0001 | ((distance * 4) << 2 & 0x03ff_fffc);
        words[1] = 0x4e80_0020;
        if let Some(slot) = words.get_mut(target) {
            *slot = 0x4e80_0020;
        }

        let image = image_of(&words, Some(BASE));
        check(&image, words.len());
    }

    /// Naming extra entries must never make blocks overlap, and must never make
    /// the same instruction belong to two of them, however the addresses fall.
    #[test]
    fn additional_entries_never_overlap_or_double_count(
        words in prop::collection::vec(any::<u32>(), 1..128),
        picks in prop::collection::vec(0u32..128, 0..8),
    ) {
        let image = image_of(&words, Some(BASE));
        let also: BTreeSet<u32> = picks
            .iter()
            .map(|pick| BASE + pick * 4)
            .collect();

        let blocks = blocks_within(&image, BASE, &BTreeSet::new(), &also);

        let mut spans: Vec<(u32, u32)> = blocks.iter().map(|b| (b.start, b.end)).collect();
        spans.sort_unstable();
        for (start, end) in &spans {
            prop_assert!(end > start, "a block must hold at least one instruction");
        }
        for pair in spans.windows(2) {
            let [(_, first_end), (second_start, _)] = pair else {
                continue;
            };
            prop_assert!(first_end <= second_start, "blocks overlap: {spans:?}");
        }

        let covered: u64 = spans.iter().map(|(s, e)| u64::from(e - s) / 4).sum();
        prop_assert!(covered <= words.len() as u64);
    }

    /// A recovered table can name an address inside the block that branches to
    /// it, so following the table leads back to the branch that named it. The
    /// alternation between walking and recovering has to settle on that rather
    /// than feeding itself.
    #[test]
    fn a_table_naming_its_own_branch_terminates(
        entries in prop::collection::vec(0u32..24, 1..8),
    ) {
        let mut words = vec![
            // cmpli r10, 7
            0x280a_0007,
            // bc to the default, four words past the branch
            0x4181_0028,
            // lis r12, 0x8200 then addi to the table at 0x82000040
            0x3d80_8200,
            0x398c_0040,
            // lbzx r0, r12, r10 then a shift left by two
            0x7c0c_50ae,
            0x5400_103a,
            // lis r12, 0x8200 then addi to a base of 0x82000000
            0x3d80_8200,
            0x398c_0000,
            // add r12, r12, r0 then move to the count register and branch
            0x7d8c_0214,
            0x7d89_03a6,
            0x4e80_0420,
        ];
        words.resize(16, 0x6000_0000);

        // The table names words of the function itself, including the ones that
        // build and take the branch.
        let mut table = 0u32;
        for (at, entry) in entries.iter().enumerate().take(4) {
            table |= entry << (24 - at * 8);
        }
        words.push(table);

        let image = image_of(&words, Some(BASE));
        check(&image, words.len());
    }
}
