//! Property tests over the parser and the address space.
//!
//! The unit tests cover the cases someone thought to write. These cover the
//! invariants that must hold across whole ranges of input, which is where an
//! off by one at a boundary tends to hide.

use proptest::prelude::*;
use xenolith_xex::{Container, Image, PageKind, Section};

/// Builds an image of `len` bytes based at `base`, all one section.
fn image_of(base: u32, len: u32) -> Image {
    let sections = vec![Section {
        start: base,
        size: len,
        kind: PageKind::Code,
    }];
    Image::new(base, vec![0xa5; len as usize], sections)
}

proptest! {
    /// Parsing must be total. Any byte sequence either parses or errors, and
    /// never panics, loops forever, or reads out of bounds.
    #[test]
    fn parsing_arbitrary_bytes_never_panics(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = Container::parse(&data);
    }

    /// The same, biased toward inputs that get past the magic check, since
    /// unbiased random bytes almost never do.
    #[test]
    fn parsing_container_shaped_bytes_never_panics(
        tail in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let mut data = b"XEX2".to_vec();
        data.extend_from_slice(&tail);
        let _ = Container::parse(&data);
    }

    /// Truncating a container at any length must error rather than panic. A
    /// short read has to be caught at the read, not by the length happening to
    /// line up.
    #[test]
    fn truncation_at_any_length_never_panics(
        tail in prop::collection::vec(any::<u8>(), 0..512),
        cut in 0usize..512,
    ) {
        let mut data = b"XEX2".to_vec();
        data.extend_from_slice(&tail);
        let cut = cut.min(data.len());
        let _ = Container::parse(&data[..cut]);
    }

    /// A read either returns exactly the requested length or fails. It must
    /// never return a short slice, which a caller would have no way to notice.
    #[test]
    fn reads_return_the_requested_length_or_fail(
        base in any::<u32>(),
        len in 0u32..4096,
        address in any::<u32>(),
        request in 0usize..8192,
    ) {
        let image = image_of(base, len);

        if let Ok(bytes) = image.read(address, request) {
            prop_assert_eq!(bytes.len(), request);
            prop_assert!(u64::from(address) >= u64::from(base));
            prop_assert!(
                u64::from(address) + request as u64 <= u64::from(base) + u64::from(len)
            );
        }
    }

    /// Sized reads agree with the byte range read of the same width.
    #[test]
    fn sized_reads_agree_with_byte_reads(
        base in any::<u32>(),
        len in 0u32..4096,
        address in any::<u32>(),
    ) {
        let image = image_of(base, len);

        prop_assert_eq!(image.u8(address).is_ok(), image.read(address, 1).is_ok());
        prop_assert_eq!(image.u16(address).is_ok(), image.read(address, 2).is_ok());
        prop_assert_eq!(image.u32(address).is_ok(), image.read(address, 4).is_ok());
        prop_assert_eq!(image.u64(address).is_ok(), image.read(address, 8).is_ok());
    }

    /// Section lookup agrees with the section bounds, in both directions.
    #[test]
    fn section_lookup_agrees_with_the_bounds(
        base in any::<u32>(),
        len in 1u32..4096,
        address in any::<u32>(),
    ) {
        let image = image_of(base, len);
        let found = image.section_at(address).is_some();
        let inside = u64::from(address) >= u64::from(base)
            && u64::from(address) < u64::from(base) + u64::from(len);

        prop_assert_eq!(found, inside);
    }
}
