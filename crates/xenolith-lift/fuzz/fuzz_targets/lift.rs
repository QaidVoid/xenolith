//! Asserts that lifting completes and stays honest over arbitrary code.
//!
//! Lifting reads whatever analysis found, which over hostile bytes is whatever
//! the walk happened to claim. Every shape has to terminate and either produce
//! C or say which instruction stopped it, never both and never neither.
//!
//! The invariant that matters most is that a function is emitted whole or not
//! at all. Code that is right except in one place compiles and runs and is
//! wrong, so an emitted function holding an instruction the model cannot
//! describe would be worse than no output.

#![no_main]

use libfuzzer_sys::fuzz_target;
use xenolith_analysis::analyze;
use xenolith_lift::{is_liftable, lift};
use xenolith_ppc::Instruction;
use xenolith_xex::{Image, PageKind, Section};

/// The address an image is placed at, matching where a title loads.
const BASE: u32 = 0x8200_0000;

/// Bounds the work one input can ask for, so a timeout means a real hang.
const MAX_BYTES: usize = 32 * 1024;

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
    // the entry point anywhere at all.
    let entry = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let image = Image::new(BASE, data.to_vec(), sections).with_entry_point(Some(entry));

    let program = analyze(&image, &[]);

    for function in program.functions() {
        let outcome = lift(&image, function);

        match outcome {
            Ok(result) => {
                assert!(
                    !result.code.is_empty(),
                    "a lifted function produced nothing"
                );
                assert!(
                    result.code.contains(&xenolith_lift::name_of(function.start)),
                    "a lifted function was not named after its address"
                );

                // Every instruction of an emitted function has to be one the
                // model can both describe and write out, or the emitted code
                // has a hole in it that nothing downstream could detect.
                for block in &function.blocks {
                    let mut address = block.start;
                    while address < block.end {
                        if let Ok(word) = image.u32(address) {
                            assert!(
                                is_liftable(Instruction::decode(word), address),
                                "an emitted function held an instruction the model cannot express"
                            );
                        }
                        address = address.saturating_add(4);
                    }
                }

                // Every address the emitted code calls has to be one it also
                // reported, or the C would name something undeclared.
                for target in &result.calls {
                    assert!(
                        result.code.contains(&xenolith_lift::name_of(*target)),
                        "a reported call does not appear in the code"
                    );
                }
            }
            Err(unlifted) => {
                assert_eq!(
                    unlifted.function, function.start,
                    "a refusal named the wrong function"
                );
                assert!(
                    !unlifted.mnemonic.is_empty(),
                    "a refusal named nothing that stopped it"
                );
            }
        }
    }
});
