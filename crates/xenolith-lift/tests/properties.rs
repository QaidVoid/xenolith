//! Property tests over lifting.
//!
//! The unit tests cover sequences someone thought to write. These cover what has
//! to hold across whole ranges of input, which for a translator is mostly one
//! thing: it never produces output it cannot stand behind. A function is emitted
//! whole or refused, and an emitted one holds nothing the model cannot express.

use proptest::prelude::*;
use xenolith_analysis::analyze;
use xenolith_lift::{Imports, is_liftable, lift};
use xenolith_ppc::Instruction;
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

proptest! {
    /// Lifting has to answer for arbitrary words, because a real title puts
    /// data and padding in its executable sections and analysis claims whatever
    /// the walk reached.
    #[test]
    fn every_function_is_lifted_whole_or_refused(
        words in prop::collection::vec(any::<u32>(), 1..256),
    ) {
        let image = image_of(&words, Some(BASE));
        let program = analyze(&image, &[]);

        for function in program.functions() {
            match lift(&image, function, &Imports::new()) {
                Ok(result) => {
                    prop_assert!(!result.code.is_empty());
                    // Nothing in an emitted function may be beyond the model.
                    for block in &function.blocks {
                        let mut address = block.start;
                        while address < block.end {
                            if let Ok(word) = image.u32(address) {
                                prop_assert!(
                                    is_liftable(Instruction::decode(word), address),
                                    "an emitted function held {} at {address:#010x}",
                                    Instruction::decode(word).opcode().mnemonic()
                                );
                            }
                            address = address.saturating_add(4);
                        }
                    }
                }
                Err(unlifted) => {
                    prop_assert_eq!(unlifted.function, function.start);
                    prop_assert!(!unlifted.mnemonic.is_empty());
                }
            }
        }
    }

    /// A refusal has to name an instruction the model really cannot express,
    /// rather than stopping on something it could have written.
    #[test]
    fn a_refusal_names_something_the_model_cannot_express(
        words in prop::collection::vec(any::<u32>(), 1..256),
    ) {
        let image = image_of(&words, Some(BASE));
        let program = analyze(&image, &[]);

        for function in program.functions() {
            let Err(unlifted) = lift(&image, function, &Imports::new()) else {
                continue;
            };
            // A function with no blocks is refused without naming an
            // instruction, since there is none to name.
            if unlifted.mnemonic == "<no blocks>" {
                prop_assert!(function.blocks.is_empty());
                continue;
            }
            if let Ok(word) = image.u32(unlifted.address) {
                prop_assert!(
                    !is_liftable(Instruction::decode(word), unlifted.address),
                    "a refusal named {} which the model can express",
                    unlifted.mnemonic
                );
            }
        }
    }

    /// Every address the emitted code names is one the emitter reported, or the
    /// C would refer to something nothing declares.
    #[test]
    fn every_call_the_code_makes_is_reported(
        words in prop::collection::vec(any::<u32>(), 1..256),
    ) {
        let image = image_of(&words, Some(BASE));
        let program = analyze(&image, &[]);

        for function in program.functions() {
            let Ok(result) = lift(&image, function, &Imports::new()) else {
                continue;
            };
            for target in &result.calls {
                prop_assert!(
                    result.code.contains(&xenolith_lift::name_of(*target)),
                    "a reported call does not appear in the code"
                );
            }
            // The other direction matters more: anything the code calls has to
            // have been reported, or it will not be declared.
            for line in result.code.lines() {
                let Some(at) = line.find("sub_") else { continue };
                let name: String = line[at..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let Some(hex) = name.strip_prefix("sub_") else {
                    continue;
                };
                let Ok(address) = u32::from_str_radix(hex, 16) else {
                    continue;
                };
                if address == function.start {
                    continue;
                }
                prop_assert!(
                    result.calls.contains(&address),
                    "the code calls {name} without reporting it"
                );
            }
        }
    }
}
