//! Asserts that decoding is total over arbitrary words.
//!
//! Instruction words come from whatever bytes sit in an executable section, and
//! a recompiler will walk over data and padding as well as code. Decoding has
//! to answer for every possible word rather than assuming it sees only valid
//! instructions.

#![no_main]

use libfuzzer_sys::fuzz_target;
use xenolith_ppc::{FlowKind, Instruction};

fuzz_target!(|data: &[u8]| {
    for chunk in data.chunks_exact(4) {
        let word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let address = word.rotate_left(9) & !3;
        let instruction = Instruction::decode(word);

        assert_eq!(instruction.word(), word, "the word must survive decoding");

        // Register fields are five bits wide and can never name more than the
        // architecture has.
        assert!(instruction.rt() < 32);
        assert!(instruction.ra() < 32);
        assert!(instruction.rb() < 32);

        // The console's extension reaches 128 vector registers and no further.
        assert!(instruction.vector_d() < 128);
        assert!(instruction.vector_a() < 128);
        assert!(instruction.vector_b() < 128);

        let flow = instruction.flow(address);
        if flow.kind == FlowKind::Continue {
            assert!(flow.falls_through, "continuing means reaching the next word");
            assert_eq!(flow.target, None, "continuing is not a transfer");
        }
        if flow.kind == FlowKind::Indirect || flow.kind == FlowKind::Return {
            assert_eq!(flow.target, None, "a register transfer claims no target");
        }

        let _ = instruction.extended_opcode();
        let _ = instruction.form();
    }
});
