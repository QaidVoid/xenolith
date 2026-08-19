//! Asserts that the full decode path is total over arbitrary bytes.
//!
//! Covers decryption, block reconstruction, and the address space view, which
//! is where a corrupt size or block length could otherwise drive an unbounded
//! allocation or an out of bounds read.

#![no_main]

use libfuzzer_sys::fuzz_target;
use xenolith_xex::{Container, KeyMaterial};

fuzz_target!(|data: &[u8]| {
    let Ok(container) = Container::parse(data) else {
        return;
    };

    let key = KeyMaterial::new([0x5a; 16]);
    let Ok(image) = container.load(Some(&key)) else {
        return;
    };

    // A decoded image must never claim more than the declared size allows.
    assert!(image.size() <= 512 * 1024 * 1024);

    let base = image.base_address();
    for offset in [0u32, 1, 4, 0xffff, 0xffff_ffff] {
        let address = base.wrapping_add(offset);
        let _ = image.read(address, 4);
        let _ = image.u32(address);
        let _ = image.section_at(address);
    }

    for section in image.sections() {
        assert!(section.end() <= u64::from(u32::MAX) + 1);
        let _ = section.permissions();
    }
});
