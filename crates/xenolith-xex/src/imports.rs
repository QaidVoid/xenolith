//! Reading what each import record in a decoded image names.
//!
//! The import libraries header lists addresses within the image and nothing
//! else. What is at those addresses says the rest: which library the record
//! belongs to, which ordinal it stands for, and whether the loader is expected
//! to write an address into it or to build a jump out of it.
//!
//! A record that does not agree with the list it was found under is rejected
//! rather than interpreted. Reading one wrongly would put a call to a different
//! import into emitted code, which nothing downstream could notice.

use alloc::borrow::ToOwned as _;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::headers::ImportLibrary;
use crate::image::Image;

/// Encoding of a record that names a value the loader writes an address into.
const KIND_SLOT: u8 = 0;

/// Encoding of the first word of a thunk.
const KIND_THUNK: u8 = 1;

/// Encoding of the second word of a thunk.
const KIND_THUNK_TAIL: u8 = 2;

/// Encoding of `mtctr r11`, the third word of a thunk.
const THUNK_MTCTR: u32 = 0x7d69_03a6;

/// Encoding of `bctr`, the fourth word of a thunk.
const THUNK_BCTR: u32 = 0x4e80_0420;

/// What an import record stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// A word the loader writes the address of the import into.
    ///
    /// Emitted code reads it from guest memory like any other data, so there is
    /// nothing to translate.
    Slot,
    /// A stub that jumps to the import.
    ///
    /// Its first two words are placeholders the loader overwrites with the
    /// halves of the resolved address, and the two after them jump through the
    /// count register to whatever those built.
    Thunk,
}

/// One import a library declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import<'a> {
    /// Name of the library the import comes from, such as `xboxkrnl.exe`.
    pub library: &'a str,
    /// Ordinal within that library.
    ///
    /// The container names imports by ordinal and never by name, so this is
    /// everything it says about which function is meant.
    pub ordinal: u16,
    /// Where the record sits in the image.
    pub address: u32,
    /// Whether the record is a slot or a thunk.
    pub kind: ImportKind,
}

/// Reads every import the libraries declare.
///
/// # Errors
///
/// Returns an error when a record address is not mapped, when a record names a
/// library other than the one that listed it, when it holds a kind this crate
/// does not recognize, or when a record declared to be a thunk does not have a
/// thunk's shape.
pub fn imports<'a>(image: &Image, libraries: &[ImportLibrary<'a>]) -> Result<Vec<Import<'a>>> {
    let mut found = Vec::new();

    for (index, library) in libraries.iter().enumerate() {
        let expected = u8::try_from(index).unwrap_or(u8::MAX);

        for address in &library.imports {
            let address = *address;
            let word = image.u32(address).map_err(|_| Error::ImportRecordOutside {
                library: library.name.to_owned(),
                address,
            })?;

            let kind = u8::try_from(word >> 24).unwrap_or(0);
            let named = u8::try_from((word >> 16) & 0xff).unwrap_or(0);
            let ordinal = u16::try_from(word & 0xffff).unwrap_or(0);

            if named != expected {
                return Err(Error::ImportRecordLibrary {
                    library: library.name.to_owned(),
                    address,
                    found: named,
                    expected,
                });
            }

            let kind = match kind {
                KIND_SLOT => ImportKind::Slot,
                KIND_THUNK => {
                    confirm_thunk(image, address, word)?;
                    ImportKind::Thunk
                }
                kind => return Err(Error::ImportRecordKind { address, kind }),
            };

            found.push(Import {
                library: library.name,
                ordinal,
                address,
                kind,
            });
        }
    }

    Ok(found)
}

/// Confirms that a record declared to be a thunk has a thunk's shape.
fn confirm_thunk(image: &Image, address: u32, word: u32) -> Result<()> {
    let malformed = || Error::ImportThunkMalformed { address };

    let tail = address.checked_add(4).ok_or_else(malformed)?;
    let expected = (word & 0x00ff_ffff) | (u32::from(KIND_THUNK_TAIL) << 24);
    if image.u32(tail).map_err(|_| malformed())? != expected {
        return Err(malformed());
    }

    let mtctr = address.checked_add(8).ok_or_else(malformed)?;
    let bctr = address.checked_add(12).ok_or_else(malformed)?;
    if image.u32(mtctr).map_err(|_| malformed())? != THUNK_MTCTR
        || image.u32(bctr).map_err(|_| malformed())? != THUNK_BCTR
    {
        return Err(malformed());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::headers::Version;
    use crate::image::Section;
    use crate::security::PageKind;

    /// Address the crafted images below load at.
    const BASE: u32 = 0x8200_0000;

    /// Builds an image holding the given words at the base address.
    fn image_of(words: &[u32]) -> Image {
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_be_bytes()).collect();
        let size = u32::try_from(bytes.len()).unwrap_or(0);
        let sections = vec![Section {
            start: BASE,
            size,
            kind: PageKind::Code,
        }];
        Image::new(BASE, bytes, sections)
    }

    /// Builds one library listing the given record addresses.
    fn library<'a>(name: &'a str, addresses: &[u32]) -> ImportLibrary<'a> {
        ImportLibrary {
            name,
            id: 0,
            version: Version(0),
            min_version: Version(0),
            imports: addresses.to_vec(),
        }
    }

    /// Returns the four words of a thunk for a library index and ordinal.
    fn thunk(index: u8, ordinal: u16) -> [u32; 4] {
        let record = (u32::from(index) << 16) | u32::from(ordinal);
        [
            (1 << 24) | record,
            (2 << 24) | record,
            THUNK_MTCTR,
            THUNK_BCTR,
        ]
    }

    #[test]
    fn reads_a_slot_and_a_thunk() {
        let mut words = vec![0x0000_0123u32];
        words.extend_from_slice(&thunk(0, 0x0123));
        let image = image_of(&words);
        let libraries = [library("xam.xex", &[BASE, BASE + 4])];

        let found = imports(&image, &libraries).expect("both records should read");

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, ImportKind::Slot);
        assert_eq!(found[0].ordinal, 0x0123);
        assert_eq!(found[0].library, "xam.xex");
        assert_eq!(found[1].kind, ImportKind::Thunk);
        assert_eq!(found[1].ordinal, 0x0123);
        assert_eq!(found[1].address, BASE + 4);
    }

    /// The second library's records have to name index one, which is what makes
    /// the reading of the middle byte a check rather than an assumption.
    #[test]
    fn the_library_index_has_to_agree() {
        let image = image_of(&[0x0000_0123]);
        let libraries = [library("xam.xex", &[]), library("xboxkrnl.exe", &[BASE])];

        let error = imports(&image, &libraries).expect_err("a mismatched index should fail");

        assert!(
            matches!(
                error,
                Error::ImportRecordLibrary {
                    address: BASE,
                    found: 0,
                    expected: 1,
                    ..
                }
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_record_outside_the_image_is_reported() {
        let image = image_of(&[0x0000_0123]);
        let libraries = [library("xam.xex", &[0x9000_0000])];

        let error = imports(&image, &libraries).expect_err("an unmapped record should fail");

        assert!(
            matches!(error, Error::ImportRecordOutside { address, .. } if address == 0x9000_0000),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_thunk_without_a_jump_is_reported() {
        let mut words = thunk(0, 0x0123).to_vec();
        words[3] = 0x6000_0000;
        let image = image_of(&words);
        let libraries = [library("xam.xex", &[BASE])];

        let error = imports(&image, &libraries).expect_err("a thunk with no jump should fail");

        assert!(
            matches!(error, Error::ImportThunkMalformed { address } if address == BASE),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_thunk_whose_second_word_disagrees_is_reported() {
        let mut words = thunk(0, 0x0123).to_vec();
        words[1] = 0x0200_0999;
        let image = image_of(&words);
        let libraries = [library("xam.xex", &[BASE])];

        let error = imports(&image, &libraries).expect_err("a mismatched tail should fail");

        assert!(
            matches!(error, Error::ImportThunkMalformed { address } if address == BASE),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_unknown_kind_is_reported() {
        let image = image_of(&[0x0700_0123]);
        let libraries = [library("xam.xex", &[BASE])];

        let error = imports(&image, &libraries).expect_err("an unknown kind should fail");

        assert!(
            matches!(error, Error::ImportRecordKind { address, kind } if address == BASE && kind == 7),
            "unexpected error: {error}"
        );
    }
}
