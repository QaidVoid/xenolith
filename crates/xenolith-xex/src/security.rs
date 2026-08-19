//! The security info block and the page descriptor table.
//!
//! Despite the name, this block is where the loader finds the shape of the
//! image: where it loads, how large it is, and how its pages are divided
//! between code and data. It also carries the encrypted session key that the
//! image body is decrypted with.
//!
//! The layout here was confirmed against a retail title. The block's declared
//! size, the end of the page descriptor array, and the start of the first
//! optional header payload all land on the same offset, and the descriptors
//! account for exactly as many pages as the declared image size holds.

use crate::error::{Error, Result};
use crate::reader::Reader;

/// Bytes occupied by one page descriptor.
const PAGE_DESCRIPTOR_SIZE: usize = 24;

/// What the pages covered by a descriptor hold.
///
/// The nibble to meaning mapping comes from the public reverse engineering
/// record. Values outside it are preserved rather than guessed at, because the
/// address space view derives section permissions from this and a wrong default
/// would be worse than an admitted unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageKind {
    /// Executable code.
    Code,
    /// Writable data.
    Data,
    /// Data that is not written after load.
    ReadOnlyData,
    /// A value this crate does not have a meaning for.
    Unknown(u8),
}

impl PageKind {
    /// Interprets the low nibble of a page descriptor.
    fn from_nibble(nibble: u8) -> Self {
        match nibble {
            1 => Self::Code,
            2 => Self::Data,
            3 => Self::ReadOnlyData,
            other => Self::Unknown(other),
        }
    }

    /// Returns whether pages of this kind hold executable code.
    #[must_use]
    pub const fn is_executable(self) -> bool {
        matches!(self, Self::Code)
    }
}

/// A run of consecutive image pages sharing one kind and one digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageDescriptor {
    /// How many pages this descriptor covers.
    pub page_count: u32,
    /// What those pages hold.
    pub kind: PageKind,
    /// Digest covering the described pages.
    pub digest: [u8; 20],
}

/// The security info block of a container.
#[derive(Debug, Clone)]
pub struct SecurityInfo {
    image_size: u32,
    image_flags: u32,
    load_address: u32,
    import_table_count: u32,
    media_id: [u8; 16],
    encrypted_session_key: [u8; 16],
    export_table_address: u32,
    game_regions: u32,
    media_flags: u32,
    page_descriptors: Vec<PageDescriptor>,
}

impl SecurityInfo {
    /// Parses the security info block at `offset` within the container.
    ///
    /// # Errors
    ///
    /// Returns an error when the offset lies outside the input, when the block
    /// is truncated, or when the declared page descriptor count does not fit in
    /// the remaining bytes.
    pub(crate) fn parse(reader: &Reader<'_>, offset: u32) -> Result<Self> {
        let mut block = reader.at(u64::from(offset), "security_info_offset")?;

        let _header_size = block.u32("security_info.header_size")?;
        let image_size = block.u32("security_info.image_size")?;
        block.skip(256, "security_info.rsa_signature")?;
        block.skip(4, "security_info.reserved")?;
        let image_flags = block.u32("security_info.image_flags")?;
        let load_address = block.u32("security_info.load_address")?;
        block.skip(20, "security_info.section_digest")?;
        let import_table_count = block.u32("security_info.import_table_count")?;
        block.skip(20, "security_info.import_table_digest")?;
        let media_id = block.take_array::<16>("security_info.media_id")?;
        let encrypted_session_key = block.take_array::<16>("security_info.session_key")?;
        let export_table_address = block.u32("security_info.export_table")?;
        block.skip(20, "security_info.header_digest")?;
        let game_regions = block.u32("security_info.game_regions")?;
        let media_flags = block.u32("security_info.media_flags")?;

        let descriptor_count = block.u32("security_info.page_descriptor_count")?;
        let page_descriptors = parse_page_descriptors(&mut block, descriptor_count)?;

        Ok(Self {
            image_size,
            image_flags,
            load_address,
            import_table_count,
            media_id,
            encrypted_session_key,
            export_table_address,
            game_regions,
            media_flags,
            page_descriptors,
        })
    }

    /// Returns the size of the image once decoded.
    #[must_use]
    pub const fn image_size(&self) -> u32 {
        self.image_size
    }

    /// Returns the raw image flags word.
    #[must_use]
    pub const fn image_flags(&self) -> u32 {
        self.image_flags
    }

    /// Returns the virtual address the image loads at.
    #[must_use]
    pub const fn load_address(&self) -> u32 {
        self.load_address
    }

    /// Returns how many import libraries the container declares.
    #[must_use]
    pub const fn import_table_count(&self) -> u32 {
        self.import_table_count
    }

    /// Returns the media identifier of the title.
    #[must_use]
    pub const fn media_id(&self) -> &[u8; 16] {
        &self.media_id
    }

    /// Returns the still encrypted session key the image body is sealed with.
    ///
    /// Unwrapping this needs key material the caller supplies, which is why it
    /// is exposed rather than resolved here.
    #[must_use]
    pub const fn encrypted_session_key(&self) -> &[u8; 16] {
        &self.encrypted_session_key
    }

    /// Returns the address of the export table, or `None` when there is none.
    #[must_use]
    pub const fn export_table_address(&self) -> Option<u32> {
        match self.export_table_address {
            0 => None,
            address => Some(address),
        }
    }

    /// Returns the raw game regions word.
    #[must_use]
    pub const fn game_regions(&self) -> u32 {
        self.game_regions
    }

    /// Returns the raw media flags word.
    #[must_use]
    pub const fn media_flags(&self) -> u32 {
        self.media_flags
    }

    /// Returns the page descriptors in the order the file stored them.
    #[must_use]
    pub fn page_descriptors(&self) -> &[PageDescriptor] {
        &self.page_descriptors
    }

    /// Returns the total number of pages the descriptors account for.
    #[must_use]
    pub fn total_pages(&self) -> u32 {
        self.page_descriptors.iter().fold(0, |total, descriptor| {
            total.saturating_add(descriptor.page_count)
        })
    }
}

/// Reads the page descriptor array, bounding it before allocating.
fn parse_page_descriptors(block: &mut Reader<'_>, count: u32) -> Result<Vec<PageDescriptor>> {
    let declared = usize::try_from(count).unwrap_or(usize::MAX);
    let needed = declared.saturating_mul(PAGE_DESCRIPTOR_SIZE);
    if needed > block.remaining() {
        return Err(Error::Truncated {
            field: "security_info.page_descriptors",
            offset: block.offset(),
            needed,
            available: block.remaining(),
        });
    }

    let mut descriptors = Vec::with_capacity(declared);
    for _ in 0..declared {
        let packed = block.u32("page_descriptor.value")?;
        let digest = block.take_array::<20>("page_descriptor.digest")?;
        descriptors.push(PageDescriptor {
            page_count: packed >> 4,
            kind: PageKind::from_nibble(u8::try_from(packed & 0xf).unwrap_or(0)),
            digest,
        });
    }

    Ok(descriptors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::Container;
    use crate::container::build::ContainerBuilder;
    use crate::error::Error;

    #[test]
    fn parses_the_descriptor_array_in_order() {
        let bytes = ContainerBuilder::new()
            .descriptors(vec![(3, 1), (5, 2), (2, 3)])
            .build();

        let container = Container::parse(&bytes).unwrap();
        let descriptors = container.security_info().page_descriptors();

        assert_eq!(descriptors.len(), 3);
        assert_eq!(descriptors[0].page_count, 3);
        assert_eq!(descriptors[0].kind, PageKind::Code);
        assert_eq!(descriptors[1].page_count, 5);
        assert_eq!(descriptors[1].kind, PageKind::Data);
        assert_eq!(descriptors[2].page_count, 2);
        assert_eq!(descriptors[2].kind, PageKind::ReadOnlyData);
    }

    /// The packing puts the page count above the kind nibble. Reading it the
    /// other way round also produces plausible looking numbers, so the split is
    /// pinned by a count that does not fit in a nibble.
    #[test]
    fn page_count_occupies_the_high_bits() {
        let bytes = ContainerBuilder::new()
            .descriptors(vec![(0x1234, 2)])
            .build();

        let container = Container::parse(&bytes).unwrap();
        let descriptor = container.security_info().page_descriptors()[0];

        assert_eq!(descriptor.page_count, 0x1234);
        assert_eq!(descriptor.kind, PageKind::Data);
    }

    #[test]
    fn unknown_page_kinds_are_preserved() {
        let bytes = ContainerBuilder::new().descriptors(vec![(1, 9)]).build();

        let container = Container::parse(&bytes).unwrap();
        let descriptor = container.security_info().page_descriptors()[0];

        assert_eq!(descriptor.kind, PageKind::Unknown(9));
        assert!(!descriptor.kind.is_executable());
    }

    #[test]
    fn only_code_pages_are_executable() {
        assert!(PageKind::Code.is_executable());
        assert!(!PageKind::Data.is_executable());
        assert!(!PageKind::ReadOnlyData.is_executable());
        assert!(!PageKind::Unknown(0).is_executable());
    }

    #[test]
    fn total_pages_sums_the_descriptors() {
        let bytes = ContainerBuilder::new()
            .descriptors(vec![(3, 1), (5, 2), (2, 3)])
            .build();

        let container = Container::parse(&bytes).unwrap();

        assert_eq!(container.security_info().total_pages(), 10);
    }

    #[test]
    fn rejects_a_descriptor_count_larger_than_the_input() {
        let bytes = ContainerBuilder::new()
            .descriptors(vec![(1, 1)])
            .declared_descriptor_count(1_000_000)
            .build();

        let error = Container::parse(&bytes).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Truncated {
                    field: "security_info.page_descriptors",
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn an_absent_export_table_is_distinct_from_a_present_one() {
        let absent = ContainerBuilder::new().export_table(0).build();
        let present = ContainerBuilder::new().export_table(0x8210_0000).build();

        let absent = Container::parse(&absent).unwrap();
        let present = Container::parse(&present).unwrap();

        assert_eq!(absent.security_info().export_table_address(), None);
        assert_eq!(
            present.security_info().export_table_address(),
            Some(0x8210_0000)
        );
    }

    #[test]
    fn reports_the_declared_import_table_count() {
        let bytes = ContainerBuilder::new().import_table_count(2).build();

        let container = Container::parse(&bytes).unwrap();

        assert_eq!(container.security_info().import_table_count(), 2);
    }

    #[test]
    fn truncating_the_security_block_errors_without_panicking() {
        let bytes = ContainerBuilder::new()
            .descriptors(vec![(1, 1), (1, 2)])
            .build();

        for length in 0..bytes.len() {
            assert!(
                Container::parse(&bytes[..length]).is_err(),
                "length {length} parsed"
            );
        }
        assert!(Container::parse(&bytes).is_ok());
    }
}
