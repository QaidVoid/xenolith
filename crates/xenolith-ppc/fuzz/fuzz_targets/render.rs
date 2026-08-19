//! Asserts that rendering is total over arbitrary words.
//!
//! Text is what a person reads when something decoded surprisingly, so it has
//! to survive exactly the inputs that are surprising.

#![no_main]

use libfuzzer_sys::fuzz_target;
use xenolith_ppc::Instruction;

fuzz_target!(|data: &[u8]| {
    for chunk in data.chunks_exact(4) {
        let word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let address = word.rotate_left(17) & !3;
        let instruction = Instruction::decode(word);

        let text = instruction.render(address).to_string();
        assert!(!text.is_empty(), "every word renders to something");

        // A word that decoded to nothing must not be given an invented name.
        if instruction.is_unknown() {
            assert!(
                text.starts_with(".long"),
                "an undecoded word was rendered as an instruction: {text}"
            );
        } else {
            assert!(
                !text.starts_with(".long"),
                "a decoded instruction was rendered as data: {text}"
            );
        }
    }
});
