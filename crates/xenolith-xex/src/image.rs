//! Reconstruction of the decoded image from a container body.
//!
//! The stored body is laid out according to the container's compression scheme.
//! The uncompressed scheme stores the image whole. The basic scheme stores a
//! sequence of blocks, each a run of bytes copied from the file followed by a
//! run of zero bytes that are not stored at all, which is how a game image full
//! of zeroed BSS pages stays small without any real compression.
//!
//! The blocks do not necessarily cover the whole image. A retail title checked
//! during development described 0xa98000 bytes across its blocks while
//! declaring an image size of 0xaa0000, so the reconstruction zero fills out to
//! the declared size rather than assuming the blocks account for all of it.

use crate::error::{Error, Result};
use crate::headers::BasicBlock;
use crate::security::{PageDescriptor, PageKind};

/// Largest image this crate will allocate for.
///
/// The console has 512 MiB of memory, so a larger image is not one it ever
/// loaded. The bound stops a corrupt size field from driving an arbitrary
/// allocation before anything has been validated.
pub(crate) const MAX_IMAGE_SIZE: u32 = 512 * 1024 * 1024;

/// Reconstructs an image stored whole.
pub(crate) fn reconstruct_uncompressed(body: &[u8], image_size: u32) -> Result<Vec<u8>> {
    let size = checked_image_size(image_size)?;

    let mut image = Vec::new();
    image
        .try_reserve(size)
        .map_err(|_| Error::ImageTooLarge { size: image_size })?;
    image.extend_from_slice(body.get(..size.min(body.len())).unwrap_or_default());
    image.resize(size, 0);

    Ok(image)
}

/// Reconstructs an image stored as basic scheme blocks.
pub(crate) fn reconstruct_basic(
    body: &[u8],
    blocks: &[BasicBlock],
    image_size: u32,
) -> Result<Vec<u8>> {
    let size = checked_image_size(image_size)?;

    let mut described: u64 = 0;
    let mut stored: u64 = 0;
    for block in blocks {
        described += u64::from(block.data_size) + u64::from(block.zero_size);
        stored += u64::from(block.data_size);
    }

    if described > u64::try_from(size).unwrap_or(u64::MAX) {
        return Err(Error::BasicBlocksExceedImage {
            described,
            image_size,
        });
    }
    if stored > u64::try_from(body.len()).unwrap_or(u64::MAX) {
        return Err(Error::BasicBlocksExceedBody {
            stored,
            available: body.len(),
        });
    }

    let mut image = Vec::new();
    image
        .try_reserve(size)
        .map_err(|_| Error::ImageTooLarge { size: image_size })?;

    let mut cursor = 0usize;
    for block in blocks {
        let data_size = usize::try_from(block.data_size).unwrap_or(usize::MAX);
        let zero_size = usize::try_from(block.zero_size).unwrap_or(usize::MAX);

        let end = cursor
            .checked_add(data_size)
            .ok_or(Error::BasicBlocksExceedBody {
                stored,
                available: body.len(),
            })?;
        let data = body.get(cursor..end).ok_or(Error::BasicBlocksExceedBody {
            stored,
            available: body.len(),
        })?;

        image.extend_from_slice(data);
        image.resize(image.len().saturating_add(zero_size), 0);
        cursor = end;
    }

    image.resize(size, 0);

    Ok(image)
}

/// Validates a declared image size and converts it to a host length.
fn checked_image_size(image_size: u32) -> Result<usize> {
    if image_size > MAX_IMAGE_SIZE {
        return Err(Error::ImageTooLarge { size: image_size });
    }
    usize::try_from(image_size).map_err(|_| Error::ImageTooLarge { size: image_size })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncompressed_reproduces_the_body() {
        let body: Vec<u8> = (0..32u8).collect();

        let image = reconstruct_uncompressed(&body, 32).unwrap();

        assert_eq!(image, body);
    }

    #[test]
    fn uncompressed_zero_fills_to_the_declared_size() {
        let body = vec![0xaa; 4];

        let image = reconstruct_uncompressed(&body, 8).unwrap();

        assert_eq!(image, vec![0xaa, 0xaa, 0xaa, 0xaa, 0, 0, 0, 0]);
    }

    #[test]
    fn basic_places_data_then_zero_fill() {
        let body = vec![1, 2, 3, 4, 5, 6];
        let blocks = [
            BasicBlock {
                data_size: 4,
                zero_size: 2,
            },
            BasicBlock {
                data_size: 2,
                zero_size: 3,
            },
        ];

        let image = reconstruct_basic(&body, &blocks, 11).unwrap();

        assert_eq!(image, vec![1, 2, 3, 4, 0, 0, 5, 6, 0, 0, 0]);
    }

    /// A retail title described fewer bytes across its blocks than its declared
    /// image size, so the tail has to be zero filled rather than assumed to be
    /// covered.
    #[test]
    fn basic_zero_fills_the_tail_beyond_the_blocks() {
        let body = vec![7, 7, 7, 7];
        let blocks = [BasicBlock {
            data_size: 4,
            zero_size: 0,
        }];

        let image = reconstruct_basic(&body, &blocks, 8).unwrap();

        assert_eq!(image, vec![7, 7, 7, 7, 0, 0, 0, 0]);
    }

    #[test]
    fn basic_with_no_blocks_yields_a_zero_image() {
        let image = reconstruct_basic(&[], &[], 6).unwrap();

        assert_eq!(image, vec![0; 6]);
    }

    #[test]
    fn rejects_blocks_describing_more_than_the_declared_image() {
        let blocks = [BasicBlock {
            data_size: 4,
            zero_size: 100,
        }];

        let error = reconstruct_basic(&[1, 2, 3, 4], &blocks, 8).unwrap_err();

        assert!(
            matches!(
                error,
                Error::BasicBlocksExceedImage {
                    described: 104,
                    image_size: 8
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_blocks_reading_past_the_stored_body() {
        let blocks = [BasicBlock {
            data_size: 64,
            zero_size: 0,
        }];

        let error = reconstruct_basic(&[1, 2, 3, 4], &blocks, 1024).unwrap_err();

        assert!(
            matches!(
                error,
                Error::BasicBlocksExceedBody {
                    stored: 64,
                    available: 4
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_an_implausible_image_size() {
        let error = reconstruct_uncompressed(&[], MAX_IMAGE_SIZE + 1).unwrap_err();

        assert!(matches!(error, Error::ImageTooLarge { .. }), "{error:?}");
    }

    #[test]
    fn accepts_a_size_just_inside_the_limit() {
        assert!(checked_image_size(MAX_IMAGE_SIZE).is_ok());
        assert!(checked_image_size(MAX_IMAGE_SIZE + 1).is_err());
    }
}

/// Bytes in one image page, as recorded by the page descriptors.
pub(crate) const PAGE_SIZE: u32 = 0x1_0000;

/// One past the highest address the console can form.
const ADDRESS_SPACE_END: u64 = u32::MAX as u64 + 1;

/// What a section permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Permissions {
    /// The section can be read.
    pub read: bool,
    /// The section can be written.
    pub write: bool,
    /// The section holds executable code.
    pub execute: bool,
}

/// A run of consecutive pages sharing one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    /// Virtual address the section starts at.
    pub start: u32,
    /// Length of the section in bytes.
    pub size: u32,
    /// What the pages of this section hold.
    pub kind: PageKind,
}

impl Section {
    /// Returns the address one past the end of the section.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.start as u64 + self.size as u64
    }

    /// Returns whether an address falls inside this section.
    #[must_use]
    pub const fn contains(&self, address: u32) -> bool {
        (address as u64) >= self.start as u64 && (address as u64) < self.end()
    }

    /// Returns what the section permits, or `None` when its kind is unknown.
    ///
    /// An unrecognized page kind yields `None` rather than a default, because a
    /// wrong guess about what is executable would mislead the stage that tells
    /// code from data.
    #[must_use]
    pub const fn permissions(&self) -> Option<Permissions> {
        match self.kind {
            PageKind::Code => Some(Permissions {
                read: true,
                write: false,
                execute: true,
            }),
            PageKind::Data => Some(Permissions {
                read: true,
                write: true,
                execute: false,
            }),
            PageKind::ReadOnlyData => Some(Permissions {
                read: true,
                write: false,
                execute: false,
            }),
            PageKind::Unknown(_) => None,
        }
    }
}

/// A decoded image, addressable by Xbox 360 virtual address.
///
/// The image loads contiguously from one base address, so a lookup is a
/// subtraction and a bounds check rather than a page table walk.
#[derive(Debug, Clone)]
pub struct Image {
    base_address: u32,
    bytes: Vec<u8>,
    sections: Vec<Section>,
    entry_point: Option<u32>,
}

impl Image {
    /// Builds an image from decoded bytes and the sections describing them.
    #[must_use]
    pub fn new(base_address: u32, bytes: Vec<u8>, sections: Vec<Section>) -> Self {
        Self {
            base_address,
            bytes,
            sections,
            entry_point: None,
        }
    }

    /// Records the entry point address.
    #[must_use]
    pub fn with_entry_point(mut self, entry_point: Option<u32>) -> Self {
        self.entry_point = entry_point;
        self
    }

    /// Coalesces page descriptors into sections of one kind each.
    ///
    /// A container can declare more pages than the 32-bit address space holds.
    /// Each section is clamped to what remains of that space and descriptors
    /// past its end are dropped, so no section can describe an address the
    /// console could not form.
    pub(crate) fn sections_from_descriptors(
        base_address: u32,
        descriptors: &[PageDescriptor],
    ) -> Vec<Section> {
        let mut sections: Vec<Section> = Vec::new();
        let mut address = u64::from(base_address);

        for descriptor in descriptors {
            let available = ADDRESS_SPACE_END.saturating_sub(address);
            if available == 0 {
                break;
            }

            let requested = u64::from(descriptor.page_count) * u64::from(PAGE_SIZE);
            let size = u32::try_from(requested.min(available).min(u64::from(u32::MAX)))
                .unwrap_or(u32::MAX);
            if size == 0 {
                continue;
            }

            match sections.last_mut() {
                Some(last) if last.kind == descriptor.kind => {
                    last.size = last.size.saturating_add(size);
                }
                _ => sections.push(Section {
                    start: u32::try_from(address).unwrap_or(u32::MAX),
                    size,
                    kind: descriptor.kind,
                }),
            }

            address += u64::from(size);
        }

        sections
    }

    /// Returns the address the image loads at.
    #[must_use]
    pub const fn base_address(&self) -> u32 {
        self.base_address
    }

    /// Returns the whole decoded image.
    ///
    /// Every other accessor here reads through a guest address. This hands the
    /// bytes over as they are, for a caller that has to put them somewhere a
    /// guest address will later index into.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the length of the image in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    /// Returns the entry point address, when the container recorded one.
    #[must_use]
    pub const fn entry_point(&self) -> Option<u32> {
        self.entry_point
    }

    /// Returns every section, in address order.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Returns the section containing `address`, if any.
    #[must_use]
    pub fn section_at(&self, address: u32) -> Option<&Section> {
        self.sections
            .iter()
            .find(|section| section.contains(address))
    }

    /// Returns only the sections holding executable code.
    pub fn executable_sections(&self) -> impl Iterator<Item = &Section> {
        self.sections
            .iter()
            .filter(|section| section.kind.is_executable())
    }

    /// Reads `len` bytes starting at a virtual address.
    ///
    /// # Errors
    ///
    /// Returns an error when the range is not fully inside the image, including
    /// when the address arithmetic would leave the 32-bit address space.
    ///
    /// ```
    /// use xenolith_xex::{Image, PageKind, Section};
    ///
    /// let sections = vec![Section { start: 0x8200_0000, size: 4, kind: PageKind::Code }];
    /// let image = Image::new(0x8200_0000, vec![0xde, 0xad, 0xbe, 0xef], sections);
    ///
    /// assert_eq!(image.read(0x8200_0001, 2).unwrap(), &[0xad, 0xbe]);
    /// assert!(image.read(0x8200_0003, 2).is_err());
    /// ```
    pub fn read(&self, address: u32, len: usize) -> Result<&[u8]> {
        let start = self.offset_of(address, len)?;
        let end = start
            .checked_add(len)
            .ok_or(Error::UnmappedRead { address, len })?;

        self.bytes
            .get(start..end)
            .ok_or(Error::UnmappedRead { address, len })
    }

    /// Reads a byte at a virtual address.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is not mapped.
    pub fn u8(&self, address: u32) -> Result<u8> {
        self.read_array::<1>(address).map(u8::from_be_bytes)
    }

    /// Reads a big-endian 16-bit value at a virtual address.
    ///
    /// # Errors
    ///
    /// Returns an error when the range is not mapped.
    pub fn u16(&self, address: u32) -> Result<u16> {
        self.read_array::<2>(address).map(u16::from_be_bytes)
    }

    /// Reads a big-endian 32-bit value at a virtual address.
    ///
    /// The console is big-endian, so this is the natural width for reading an
    /// instruction or a pointer out of the image.
    ///
    /// # Errors
    ///
    /// Returns an error when the range is not mapped.
    ///
    /// ```
    /// use xenolith_xex::{Image, PageKind, Section};
    ///
    /// let sections = vec![Section { start: 0x8200_0000, size: 4, kind: PageKind::Code }];
    /// let image = Image::new(0x8200_0000, vec![0x7c, 0x08, 0x02, 0xa6], sections);
    ///
    /// assert_eq!(image.u32(0x8200_0000).unwrap(), 0x7c08_02a6);
    /// ```
    pub fn u32(&self, address: u32) -> Result<u32> {
        self.read_array::<4>(address).map(u32::from_be_bytes)
    }

    /// Reads a big-endian 64-bit value at a virtual address.
    ///
    /// # Errors
    ///
    /// Returns an error when the range is not mapped.
    pub fn u64(&self, address: u32) -> Result<u64> {
        self.read_array::<8>(address).map(u64::from_be_bytes)
    }

    /// Reads a fixed width value at a virtual address.
    fn read_array<const N: usize>(&self, address: u32) -> Result<[u8; N]> {
        let bytes = self.read(address, N)?;
        <[u8; N]>::try_from(bytes).map_err(|_| Error::UnmappedRead { address, len: N })
    }

    /// Converts a virtual address to an offset, rejecting unmapped ranges.
    fn offset_of(&self, address: u32, len: usize) -> Result<usize> {
        let unmapped = || Error::UnmappedRead { address, len };

        let length = u64::try_from(len).unwrap_or(u64::MAX);
        let last = u64::from(address)
            .checked_add(length)
            .ok_or_else(unmapped)?;
        if last > u64::from(u32::MAX) + 1 {
            return Err(unmapped());
        }

        let offset = u64::from(address)
            .checked_sub(u64::from(self.base_address))
            .ok_or_else(unmapped)?;

        usize::try_from(offset).map_err(|_| unmapped())
    }
}

#[cfg(test)]
mod address_space_tests {
    use super::*;

    /// Builds an image of `len` ascending bytes based at 0x82000000.
    fn image_of(len: usize, kind: PageKind) -> Image {
        let bytes: Vec<u8> = (0..len)
            .map(|index| u8::try_from(index % 251).unwrap_or(0))
            .collect();
        let sections = vec![Section {
            start: 0x8200_0000,
            size: u32::try_from(len).unwrap(),
            kind,
        }];
        Image::new(0x8200_0000, bytes, sections)
    }

    #[test]
    fn reads_inside_a_section() {
        let image = image_of(16, PageKind::Code);

        assert_eq!(image.read(0x8200_0004, 4).unwrap(), &[4, 5, 6, 7]);
    }

    #[test]
    fn reads_big_endian_widths() {
        let sections = vec![Section {
            start: 0x8200_0000,
            size: 8,
            kind: PageKind::Code,
        }];
        let image = Image::new(0x8200_0000, vec![1, 2, 3, 4, 5, 6, 7, 8], sections);

        assert_eq!(image.u8(0x8200_0000).unwrap(), 1);
        assert_eq!(image.u16(0x8200_0000).unwrap(), 0x0102);
        assert_eq!(image.u32(0x8200_0000).unwrap(), 0x0102_0304);
        assert_eq!(image.u64(0x8200_0000).unwrap(), 0x0102_0304_0506_0708);
    }

    #[test]
    fn rejects_a_read_below_the_base() {
        let image = image_of(16, PageKind::Code);

        let error = image.read(0x81ff_ffff, 1).unwrap_err();

        assert!(matches!(error, Error::UnmappedRead { .. }), "{error:?}");
    }

    #[test]
    fn rejects_a_read_straddling_the_end() {
        let image = image_of(16, PageKind::Code);

        assert!(image.read(0x8200_000e, 2).is_ok());
        assert!(image.read(0x8200_000f, 2).is_err());
    }

    #[test]
    fn rejects_a_sized_read_too_close_to_the_end() {
        let image = image_of(16, PageKind::Code);

        assert!(image.u32(0x8200_000c).is_ok());
        assert!(image.u32(0x8200_000d).is_err());
    }

    /// A read near the top of the address space must fail rather than wrap.
    ///
    /// This image ends exactly at the top of the 32-bit space, so a read that
    /// finishes on that boundary is legal and only one going past it is not.
    #[test]
    fn rejects_arithmetic_that_would_leave_the_address_space() {
        let sections = vec![Section {
            start: 0xffff_fff0,
            size: 16,
            kind: PageKind::Data,
        }];
        let image = Image::new(0xffff_fff0, vec![0; 16], sections);

        assert!(image.read(0xffff_fff0, 16).is_ok());
        assert!(image.u64(0xffff_fff8).is_ok());

        assert!(image.read(0xffff_ffff, 8).is_err());
        assert!(image.u64(0xffff_fff9).is_err());
        assert!(image.read(0xffff_fff0, 17).is_err());
    }

    #[test]
    fn a_zero_length_read_inside_the_image_succeeds() {
        let image = image_of(16, PageKind::Code);

        assert_eq!(image.read(0x8200_0004, 0).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn resolves_an_address_to_its_section() {
        let descriptors = [
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 2,
                kind: PageKind::Data,
                digest: [0; 20],
            },
        ];
        let sections = Image::sections_from_descriptors(0x8200_0000, &descriptors);
        let image = Image::new(0x8200_0000, vec![0; 3 * 0x1_0000], sections);

        assert_eq!(image.section_at(0x8200_0000).unwrap().kind, PageKind::Code);
        assert_eq!(image.section_at(0x8201_0000).unwrap().kind, PageKind::Data);
        assert_eq!(image.section_at(0x8202_ffff).unwrap().kind, PageKind::Data);
        assert!(image.section_at(0x8203_0000).is_none());
    }

    #[test]
    fn adjacent_descriptors_of_one_kind_become_a_single_section() {
        let descriptors = [
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 3,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Data,
                digest: [0; 20],
            },
        ];

        let sections = Image::sections_from_descriptors(0x8200_0000, &descriptors);

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].size, 4 * PAGE_SIZE);
        assert_eq!(sections[0].kind, PageKind::Code);
        assert_eq!(sections[1].start, 0x8204_0000);
    }

    #[test]
    fn sections_do_not_overlap() {
        let descriptors = [
            PageDescriptor {
                page_count: 2,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Data,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 1,
                kind: PageKind::ReadOnlyData,
                digest: [0; 20],
            },
        ];

        let sections = Image::sections_from_descriptors(0x8200_0000, &descriptors);

        for pair in sections.windows(2) {
            assert!(
                pair[0].end() <= u64::from(pair[1].start),
                "sections overlap"
            );
        }
    }

    /// Found by fuzzing the decode path. A container can declare more pages
    /// than the address space holds, and clamping each section to what remains
    /// is what keeps the table describing addresses the console could form.
    #[test]
    fn sections_never_extend_past_the_address_space() {
        let descriptors = [
            PageDescriptor {
                page_count: 0x0fff_ffff,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 0x0fff_ffff,
                kind: PageKind::Data,
                digest: [0; 20],
            },
        ];

        for base in [0u32, 0x8200_0000, 0xf000_0000, 0xffff_0000] {
            let sections = Image::sections_from_descriptors(base, &descriptors);

            for section in &sections {
                assert!(
                    section.end() <= u64::from(u32::MAX) + 1,
                    "base {base:#x} produced {section:?} past the address space"
                );
                assert!(section.start >= base, "section starts below the base");
            }
        }
    }

    #[test]
    fn descriptors_beyond_the_address_space_are_dropped() {
        let descriptors = [
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Data,
                digest: [0; 20],
            },
        ];

        let sections = Image::sections_from_descriptors(0xffff_0000, &descriptors);

        assert_eq!(sections.len(), 1, "a section past the end survived");
        assert_eq!(sections[0].kind, PageKind::Code);
        assert_eq!(sections[0].end(), u64::from(u32::MAX) + 1);
    }

    #[test]
    fn zero_page_descriptors_produce_no_section() {
        let descriptors = [
            PageDescriptor {
                page_count: 0,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Data,
                digest: [0; 20],
            },
        ];

        let sections = Image::sections_from_descriptors(0x8200_0000, &descriptors);

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, PageKind::Data);
        assert_eq!(sections[0].start, 0x8200_0000);
    }

    #[test]
    fn only_code_sections_are_executable() {
        let descriptors = [
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Code,
                digest: [0; 20],
            },
            PageDescriptor {
                page_count: 1,
                kind: PageKind::Data,
                digest: [0; 20],
            },
        ];
        let sections = Image::sections_from_descriptors(0x8200_0000, &descriptors);
        let image = Image::new(0x8200_0000, vec![0; 2 * 0x1_0000], sections);

        let executable: Vec<_> = image.executable_sections().collect();

        assert_eq!(executable.len(), 1);
        assert_eq!(executable[0].kind, PageKind::Code);
    }

    #[test]
    fn permissions_follow_the_page_kind() {
        let code = Section {
            start: 0,
            size: 1,
            kind: PageKind::Code,
        };
        let data = Section {
            start: 0,
            size: 1,
            kind: PageKind::Data,
        };
        let readonly = Section {
            start: 0,
            size: 1,
            kind: PageKind::ReadOnlyData,
        };

        assert_eq!(
            code.permissions(),
            Some(Permissions {
                read: true,
                write: false,
                execute: true
            })
        );
        assert_eq!(
            data.permissions(),
            Some(Permissions {
                read: true,
                write: true,
                execute: false
            })
        );
        assert_eq!(
            readonly.permissions(),
            Some(Permissions {
                read: true,
                write: false,
                execute: false
            })
        );
    }

    /// An unknown kind reports no permissions rather than a default, so a later
    /// stage cannot silently treat it as data.
    #[test]
    fn an_unknown_page_kind_reports_no_permissions() {
        let section = Section {
            start: 0,
            size: 1,
            kind: PageKind::Unknown(7),
        };

        assert_eq!(section.permissions(), None);
    }
}
