//! Typed readings of the optional headers this crate understands.
//!
//! The container keeps every optional header as a key and a span of bytes. This
//! module turns the ones the loader depends on into real values, and leaves the
//! rest alone.
//!
//! Every layout here was confirmed against a retail title. Where a reading
//! could not be confirmed it is preserved raw rather than guessed at.

use crate::error::{Error, Result};
use crate::reader::Reader;

/// Keys of the optional headers this crate reads.
pub mod keys {
    /// Embedded resource directory.
    pub const RESOURCE_INFO: u32 = 0x0000_02ff;
    /// Encryption and compression of the image body.
    pub const FILE_FORMAT_INFO: u32 = 0x0000_03ff;
    /// Virtual address execution begins at.
    pub const ENTRY_POINT: u32 = 0x0001_0100;
    /// Virtual address the image loads at.
    pub const IMAGE_BASE_ADDRESS: u32 = 0x0001_0201;
    /// Libraries the image imports from.
    pub const IMPORT_LIBRARIES: u32 = 0x0001_03ff;
    /// Title identity and version.
    pub const EXECUTION_INFO: u32 = 0x0004_0006;
}

/// A packed version, holding a major, minor, build, and QFE component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32);

impl Version {
    /// Returns the major component.
    #[must_use]
    pub const fn major(self) -> u8 {
        ((self.0 >> 28) & 0xf) as u8
    }

    /// Returns the minor component.
    #[must_use]
    pub const fn minor(self) -> u8 {
        ((self.0 >> 24) & 0xf) as u8
    }

    /// Returns the build component.
    #[must_use]
    pub const fn build(self) -> u16 {
        ((self.0 >> 8) & 0xffff) as u16
    }

    /// Returns the QFE component.
    #[must_use]
    pub const fn qfe(self) -> u8 {
        (self.0 & 0xff) as u8
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major(),
            self.minor(),
            self.build(),
            self.qfe()
        )
    }
}

/// Whether the image body is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionType {
    /// Stored in the clear.
    None,
    /// Sealed with the session key held in the security info.
    Encrypted,
    /// A value this crate does not have a meaning for.
    Unknown(u16),
}

/// How the image body is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    /// Stored whole.
    None,
    /// A sequence of raw blocks, each followed by a run of zero bytes.
    Basic,
    /// LZX compressed blocks.
    Normal,
    /// A delta against another image, used by title updates.
    Delta,
    /// A value this crate does not have a meaning for.
    Unknown(u16),
}

/// One block of a basic scheme image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicBlock {
    /// Bytes copied from the file.
    pub data_size: u32,
    /// Zero bytes appended after the copied data.
    pub zero_size: u32,
}

/// Parameters of an LZX compressed image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalCompression {
    /// LZX window size the stream was produced with.
    pub window_size: u32,
    /// Size of the first block in the chain.
    pub first_block_size: u32,
    /// Digest of the first block.
    pub first_block_digest: [u8; 20],
}

/// The layout of the image body within the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFormatInfo {
    encryption: EncryptionType,
    compression: CompressionType,
    basic_blocks: Vec<BasicBlock>,
    normal: Option<NormalCompression>,
}

impl FileFormatInfo {
    /// Parses the file format info payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is truncated or declares a size
    /// smaller than its own fixed fields.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);

        let info_size = reader.u32("file_format_info.info_size")?;
        let encryption = match reader.u16("file_format_info.encryption_type")? {
            0 => EncryptionType::None,
            1 => EncryptionType::Encrypted,
            other => EncryptionType::Unknown(other),
        };
        let compression = match reader.u16("file_format_info.compression_type")? {
            0 => CompressionType::None,
            1 => CompressionType::Basic,
            2 => CompressionType::Normal,
            3 => CompressionType::Delta,
            other => CompressionType::Unknown(other),
        };

        let mut info = Self {
            encryption,
            compression,
            basic_blocks: Vec::new(),
            normal: None,
        };

        match compression {
            CompressionType::Basic => {
                let declared = usize::try_from(info_size).unwrap_or(usize::MAX);
                let block_bytes = declared.saturating_sub(8);
                if block_bytes % 8 != 0 {
                    return Err(Error::FileFormatInfoInvalidSize { size: info_size });
                }
                for _ in 0..block_bytes / 8 {
                    info.basic_blocks.push(BasicBlock {
                        data_size: reader.u32("basic_block.data_size")?,
                        zero_size: reader.u32("basic_block.zero_size")?,
                    });
                }
            }
            CompressionType::Normal => {
                info.normal = Some(NormalCompression {
                    window_size: reader.u32("normal.window_size")?,
                    first_block_size: reader.u32("normal.first_block_size")?,
                    first_block_digest: reader.take_array::<20>("normal.first_block_digest")?,
                });
            }
            _ => {}
        }

        Ok(info)
    }

    /// Returns whether the image body is encrypted.
    #[must_use]
    pub const fn encryption(&self) -> EncryptionType {
        self.encryption
    }

    /// Returns how the image body is compressed.
    #[must_use]
    pub const fn compression(&self) -> CompressionType {
        self.compression
    }

    /// Returns the blocks of a basic scheme image, empty for other schemes.
    #[must_use]
    pub fn basic_blocks(&self) -> &[BasicBlock] {
        &self.basic_blocks
    }

    /// Returns the LZX parameters, or `None` for other schemes.
    #[must_use]
    pub const fn normal(&self) -> Option<NormalCompression> {
        self.normal
    }
}

/// Identity and version of the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionInfo {
    /// Identifier of the media the title shipped on.
    pub media_id: u32,
    /// Version of this build.
    pub version: Version,
    /// Version this build updates from.
    pub base_version: Version,
    /// Identifier of the title.
    pub title_id: u32,
    /// Platform the title targets.
    pub platform: u8,
    /// Index of the executable within the title.
    pub executable_table: u8,
    /// Which disc this is.
    pub disc_number: u8,
    /// How many discs the title shipped on.
    pub disc_count: u8,
    /// Identifier used for save data.
    pub savegame_id: u32,
}

impl ExecutionInfo {
    /// Parses the execution info payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is shorter than the structure.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);

        let media_id = reader.u32("execution_info.media_id")?;
        let version = Version(reader.u32("execution_info.version")?);
        let base_version = Version(reader.u32("execution_info.base_version")?);
        let title_id = reader.u32("execution_info.title_id")?;
        let flags = reader.take_array::<4>("execution_info.flags")?;
        let savegame_id = reader.u32("execution_info.savegame_id")?;

        let [platform, executable_table, disc_number, disc_count] = flags;

        Ok(Self {
            media_id,
            version,
            base_version,
            title_id,
            platform,
            executable_table,
            disc_number,
            disc_count,
            savegame_id,
        })
    }
}

/// A library the image imports from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLibrary<'a> {
    /// Name of the library, such as `xboxkrnl.exe`.
    pub name: &'a str,
    /// Identifier of the library.
    pub id: u32,
    /// Version the image was built against.
    pub version: Version,
    /// Oldest version the image accepts.
    pub min_version: Version,
    /// Virtual addresses of the import records within the image.
    pub imports: Vec<u32>,
}

/// Parses the import libraries payload.
///
/// # Errors
///
/// Returns an error when the payload is truncated, when the string table does
/// not hold the names the records index, or when a record's declared size does
/// not match the imports it claims.
pub(crate) fn parse_import_libraries(bytes: &[u8]) -> Result<Vec<ImportLibrary<'_>>> {
    /// Bytes of a library record before its import addresses.
    const RECORD_HEADER_SIZE: u32 = 40;

    let mut reader = Reader::new(bytes);

    let _total_size = reader.u32("import_libraries.total_size")?;
    let string_table_size = reader.u32("import_libraries.string_table_size")?;
    let library_count = reader.u32("import_libraries.library_count")?;

    let table_bytes = usize::try_from(string_table_size).unwrap_or(usize::MAX);
    let string_table = reader.take(table_bytes, "import_libraries.string_table")?;
    let names: Vec<&str> = string_table
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .filter_map(|name| core::str::from_utf8(name).ok())
        .collect();

    let declared = usize::try_from(library_count).unwrap_or(usize::MAX);
    let mut libraries = Vec::with_capacity(declared.min(reader.remaining() / 40));

    for _ in 0..declared {
        let start = reader.offset();
        let size = reader.u32("import_library.size")?;
        reader.skip(20, "import_library.next_import_digest")?;
        let id = reader.u32("import_library.id")?;
        let version = Version(reader.u32("import_library.version")?);
        let min_version = Version(reader.u32("import_library.min_version")?);
        let name_index = reader.u16("import_library.name_index")?;
        let import_count = reader.u16("import_library.import_count")?;

        if size != RECORD_HEADER_SIZE.saturating_add(u32::from(import_count).saturating_mul(4)) {
            return Err(Error::ImportLibraryInvalidSize {
                offset: start,
                size,
            });
        }

        let name =
            names
                .get(usize::from(name_index))
                .copied()
                .ok_or(Error::ImportLibraryNameIndex {
                    index: name_index,
                    count: names.len(),
                })?;

        let mut imports = Vec::with_capacity(usize::from(import_count));
        for _ in 0..import_count {
            imports.push(reader.u32("import_library.import")?);
        }

        libraries.push(ImportLibrary {
            name,
            id,
            version,
            min_version,
            imports,
        });
    }

    Ok(libraries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a file format info payload with the given fixed fields and tail.
    fn format_payload(encryption: u16, compression: u16, tail: &[u8]) -> Vec<u8> {
        let size = u32::try_from(8 + tail.len()).unwrap();
        let mut bytes = size.to_be_bytes().to_vec();
        bytes.extend_from_slice(&encryption.to_be_bytes());
        bytes.extend_from_slice(&compression.to_be_bytes());
        bytes.extend_from_slice(tail);
        bytes
    }

    /// Builds an import libraries payload from names and per library imports.
    fn import_payload(names: &[&str], libraries: &[(u16, &[u32])]) -> Vec<u8> {
        let mut table = Vec::new();
        for name in names {
            table.extend_from_slice(name.as_bytes());
            table.push(0);
        }
        while table.len() % 4 != 0 {
            table.push(0);
        }

        let mut records = Vec::new();
        for (name_index, imports) in libraries {
            let size = u32::try_from(40 + imports.len() * 4).unwrap();
            records.extend_from_slice(&size.to_be_bytes());
            records.extend_from_slice(&[0u8; 20]);
            records.extend_from_slice(&0xabcd_1234u32.to_be_bytes());
            records.extend_from_slice(&0x2017_f400u32.to_be_bytes());
            records.extend_from_slice(&0x2016_9b00u32.to_be_bytes());
            records.extend_from_slice(&name_index.to_be_bytes());
            records.extend_from_slice(&u16::try_from(imports.len()).unwrap().to_be_bytes());
            for import in *imports {
                records.extend_from_slice(&import.to_be_bytes());
            }
        }

        let total = u32::try_from(12 + table.len() + records.len()).unwrap();
        let mut bytes = total.to_be_bytes().to_vec();
        bytes.extend_from_slice(&u32::try_from(table.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&u32::try_from(libraries.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&table);
        bytes.extend_from_slice(&records);
        bytes
    }

    #[test]
    fn decodes_a_packed_version() {
        let version = Version(0x2017_f400);

        assert_eq!(version.major(), 2);
        assert_eq!(version.minor(), 0);
        assert_eq!(version.build(), 0x17f4);
        assert_eq!(version.qfe(), 0);
        assert_eq!(version.to_string(), "2.0.6132.0");
    }

    #[test]
    fn reads_an_uncompressed_unencrypted_image() {
        let info = FileFormatInfo::parse(&format_payload(0, 0, &[])).unwrap();

        assert_eq!(info.encryption(), EncryptionType::None);
        assert_eq!(info.compression(), CompressionType::None);
        assert!(info.basic_blocks().is_empty());
        assert_eq!(info.normal(), None);
    }

    #[test]
    fn reads_basic_scheme_blocks() {
        let mut tail = Vec::new();
        for (data, zero) in [(0x3e_8000u32, 0x8000u32), (0x30_0000, 0x36_8000)] {
            tail.extend_from_slice(&data.to_be_bytes());
            tail.extend_from_slice(&zero.to_be_bytes());
        }

        let info = FileFormatInfo::parse(&format_payload(1, 1, &tail)).unwrap();

        assert_eq!(info.encryption(), EncryptionType::Encrypted);
        assert_eq!(info.compression(), CompressionType::Basic);
        assert_eq!(
            info.basic_blocks(),
            &[
                BasicBlock {
                    data_size: 0x3e_8000,
                    zero_size: 0x8000
                },
                BasicBlock {
                    data_size: 0x30_0000,
                    zero_size: 0x36_8000
                },
            ]
        );
    }

    #[test]
    fn reads_normal_scheme_parameters() {
        let mut tail = 0x0002_0000u32.to_be_bytes().to_vec();
        tail.extend_from_slice(&0x8000u32.to_be_bytes());
        tail.extend_from_slice(&[7u8; 20]);

        let info = FileFormatInfo::parse(&format_payload(0, 2, &tail)).unwrap();

        assert_eq!(info.compression(), CompressionType::Normal);
        assert_eq!(
            info.normal(),
            Some(NormalCompression {
                window_size: 0x0002_0000,
                first_block_size: 0x8000,
                first_block_digest: [7u8; 20],
            })
        );
        assert!(info.basic_blocks().is_empty());
    }

    #[test]
    fn preserves_unrecognized_scheme_values() {
        let info = FileFormatInfo::parse(&format_payload(9, 7, &[])).unwrap();

        assert_eq!(info.encryption(), EncryptionType::Unknown(9));
        assert_eq!(info.compression(), CompressionType::Unknown(7));
    }

    #[test]
    fn rejects_a_basic_size_that_is_not_whole_blocks() {
        let error = FileFormatInfo::parse(&format_payload(0, 1, &[0u8; 4])).unwrap_err();

        assert!(
            matches!(error, Error::FileFormatInfoInvalidSize { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn reads_execution_info() {
        let mut bytes = 0x0379_efb3u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(&0x0000_000bu32.to_be_bytes());
        bytes.extend_from_slice(&0x0000_000au32.to_be_bytes());
        bytes.extend_from_slice(&0x4e4d_07d1u32.to_be_bytes());
        bytes.extend_from_slice(&[0, 0, 1, 2]);
        bytes.extend_from_slice(&0u32.to_be_bytes());

        let info = ExecutionInfo::parse(&bytes).unwrap();

        assert_eq!(info.media_id, 0x0379_efb3);
        assert_eq!(info.version, Version(0x0b));
        assert_eq!(info.base_version, Version(0x0a));
        assert_eq!(info.title_id, 0x4e4d_07d1);
        assert_eq!(info.disc_number, 1);
        assert_eq!(info.disc_count, 2);
    }

    #[test]
    fn rejects_execution_info_shorter_than_the_structure() {
        assert!(ExecutionInfo::parse(&[0u8; 20]).is_err());
    }

    #[test]
    fn reads_import_libraries() {
        let payload = import_payload(
            &["xam.xex", "xboxkrnl.exe"],
            &[(0, &[0x8200_1000, 0x8200_1004]), (1, &[0x8200_2000])],
        );

        let libraries = parse_import_libraries(&payload).unwrap();

        assert_eq!(libraries.len(), 2);
        assert_eq!(libraries[0].name, "xam.xex");
        assert_eq!(libraries[0].imports, vec![0x8200_1000, 0x8200_1004]);
        assert_eq!(libraries[0].version, Version(0x2017_f400));
        assert_eq!(libraries[0].min_version, Version(0x2016_9b00));
        assert_eq!(libraries[1].name, "xboxkrnl.exe");
        assert_eq!(libraries[1].imports, vec![0x8200_2000]);
    }

    #[test]
    fn reads_a_library_with_no_imports() {
        let payload = import_payload(&["xam.xex"], &[(0, &[])]);

        let libraries = parse_import_libraries(&payload).unwrap();

        assert_eq!(libraries.len(), 1);
        assert!(libraries[0].imports.is_empty());
    }

    #[test]
    fn rejects_a_name_index_outside_the_string_table() {
        let payload = import_payload(&["xam.xex"], &[(5, &[0x8200_1000])]);

        let error = parse_import_libraries(&payload).unwrap_err();

        assert_eq!(error, Error::ImportLibraryNameIndex { index: 5, count: 1 });
    }

    /// A record's size is fully determined by its import count, so the two
    /// disagreeing means the record was misread and walking on would compound
    /// the error into every library that follows.
    #[test]
    fn rejects_a_record_size_that_disagrees_with_its_import_count() {
        let mut payload = import_payload(&["xam.xex"], &[(0, &[0x8200_1000])]);
        let record = 12 + 8;
        payload[record..record + 4].copy_from_slice(&0x99u32.to_be_bytes());

        let error = parse_import_libraries(&payload).unwrap_err();

        assert!(
            matches!(error, Error::ImportLibraryInvalidSize { size: 0x99, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn truncating_an_import_payload_errors_without_panicking() {
        let payload = import_payload(&["xam.xex"], &[(0, &[0x8200_1000])]);

        for length in 0..payload.len() {
            assert!(
                parse_import_libraries(&payload[..length]).is_err(),
                "length {length} parsed"
            );
        }
        assert!(parse_import_libraries(&payload).is_ok());
    }
}
