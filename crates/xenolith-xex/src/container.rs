//! The XEX file header and its optional header directory.
//!
//! A container opens with a fixed header naming the format, the module flags,
//! and the offsets of the image data and the security info. Everything else is
//! reached through the optional header directory that follows it, a table of
//! key and value pairs where the key identifies the kind of data and also
//! encodes how to reach it.
//!
//! Parsing here is deliberately shallow. Entries are located, bounded, and
//! kept, but their contents are not interpreted, so an unrecognized key costs
//! nothing and is preserved for callers rather than discarded.

use crate::error::{Error, Result};
use crate::headers::{ExecutionInfo, FileFormatInfo, ImportLibrary, keys, parse_import_libraries};
use crate::reader::Reader;
use crate::security::SecurityInfo;

/// Container formats this crate recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// The format used by retail Xbox 360 titles.
    Xex2,
    /// An earlier revision sharing the same overall shape.
    Xex1,
}

impl Format {
    /// The four byte magic that identifies this format.
    #[must_use]
    pub const fn magic(self) -> [u8; 4] {
        match self {
            Self::Xex2 => *b"XEX2",
            Self::Xex1 => *b"XEX1",
        }
    }

    /// Identifies a format from the first four bytes of a file.
    fn from_magic(magic: [u8; 4]) -> Result<Self> {
        match &magic {
            b"XEX2" => Ok(Self::Xex2),
            b"XEX1" => Ok(Self::Xex1),
            _ => Err(Error::UnsupportedMagic { magic }),
        }
    }
}

/// Identifier of an optional header entry.
///
/// The upper bits name the kind of data. The low byte encodes how to reach it,
/// which is why the key alone is enough to locate an entry whose meaning is
/// unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionalHeaderKey(pub u32);

/// How the data belonging to an optional header entry is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Storage {
    /// The entry's value field is the data.
    Inline,
    /// The value field is a file offset to this many 32-bit words of data.
    Fixed(u32),
    /// The value field is a file offset to data that opens with its own
    /// 32-bit total length, counting that length field.
    Variable,
}

impl OptionalHeaderKey {
    /// Returns how the data for this entry is stored.
    ///
    /// This mapping comes from the public reverse engineering record for the
    /// format rather than from a published specification, so it is one of the
    /// parts of this crate that most wants checking against real files.
    fn storage(self) -> Storage {
        match self.0 & 0xff {
            0x00 | 0x01 => Storage::Inline,
            0xff => Storage::Variable,
            words => Storage::Fixed(words),
        }
    }
}

/// The data belonging to an optional header entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionalHeaderValue<'a> {
    /// The entry carried its data in the directory itself.
    Inline(u32),
    /// The entry pointed at data elsewhere in the file.
    Data(&'a [u8]),
}

impl<'a> OptionalHeaderValue<'a> {
    /// Returns the inline value, or `None` when the entry pointed elsewhere.
    #[must_use]
    pub const fn inline(self) -> Option<u32> {
        match self {
            Self::Inline(value) => Some(value),
            Self::Data(_) => None,
        }
    }

    /// Returns the referenced bytes, or `None` when the entry was inline.
    #[must_use]
    pub const fn data(self) -> Option<&'a [u8]> {
        match self {
            Self::Data(bytes) => Some(bytes),
            Self::Inline(_) => None,
        }
    }
}

/// One entry of the optional header directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionalHeader<'a> {
    /// Identifier naming the kind of data and how it is stored.
    pub key: OptionalHeaderKey,
    /// The entry's data, either inline or resolved from elsewhere in the file.
    pub value: OptionalHeaderValue<'a>,
}

/// A parsed XEX container header.
///
/// Borrows the input so that referenced data costs no copies. Nothing beyond
/// the header and the directory has been interpreted.
#[derive(Debug, Clone)]
pub struct Container<'a> {
    format: Format,
    module_flags: u32,
    image_offset: u32,
    security_info_offset: u32,
    optional_headers: Vec<OptionalHeader<'a>>,
    security_info: SecurityInfo,
    file_format_info: Option<FileFormatInfo>,
    execution_info: Option<ExecutionInfo>,
    import_libraries: Vec<ImportLibrary<'a>>,
    entry_point: Option<u32>,
    image_base_address: Option<u32>,
}

impl<'a> Container<'a> {
    /// Parses the header and optional header directory of a container.
    ///
    /// # Errors
    ///
    /// Returns an error when the magic is unrecognized, when the input is too
    /// short for a field being read, or when an optional header entry points
    /// outside the input.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);

        let format = Format::from_magic(read_magic(&mut reader)?)?;
        let module_flags = reader.u32("module_flags")?;
        let image_offset = reader.u32("image_offset")?;
        reader.skip(4, "reserved")?;
        let security_info_offset = reader.u32("security_info_offset")?;
        let optional_header_count = reader.u32("optional_header_count")?;

        let optional_headers = parse_optional_headers(&mut reader, optional_header_count)?;
        let security_info = SecurityInfo::parse(&reader, security_info_offset)?;

        let payload = |key: u32| {
            optional_headers
                .iter()
                .find(|header| header.key.0 == key)
                .map(|header| header.value)
        };

        let file_format_info =
            match payload(keys::FILE_FORMAT_INFO).and_then(OptionalHeaderValue::data) {
                Some(data) => Some(FileFormatInfo::parse(data)?),
                None => None,
            };
        let execution_info = match payload(keys::EXECUTION_INFO).and_then(OptionalHeaderValue::data)
        {
            Some(data) => Some(ExecutionInfo::parse(data)?),
            None => None,
        };
        let import_libraries =
            match payload(keys::IMPORT_LIBRARIES).and_then(OptionalHeaderValue::data) {
                Some(data) => parse_import_libraries(data)?,
                None => Vec::new(),
            };

        let entry_point = payload(keys::ENTRY_POINT).and_then(OptionalHeaderValue::inline);
        let image_base_address =
            payload(keys::IMAGE_BASE_ADDRESS).and_then(OptionalHeaderValue::inline);

        Ok(Self {
            format,
            module_flags,
            image_offset,
            security_info_offset,
            optional_headers,
            security_info,
            file_format_info,
            execution_info,
            import_libraries,
            entry_point,
            image_base_address,
        })
    }

    /// Returns the security info block, which describes the image layout.
    #[must_use]
    pub const fn security_info(&self) -> &SecurityInfo {
        &self.security_info
    }

    /// Returns how the image body is encrypted and compressed.
    ///
    /// Absent only for containers that declare no file format info, which no
    /// retail title is expected to do.
    #[must_use]
    pub const fn file_format_info(&self) -> Option<&FileFormatInfo> {
        self.file_format_info.as_ref()
    }

    /// Returns the title identity and version, when the container carries it.
    #[must_use]
    pub const fn execution_info(&self) -> Option<ExecutionInfo> {
        self.execution_info
    }

    /// Returns the libraries the image imports from.
    #[must_use]
    pub fn import_libraries(&self) -> &[ImportLibrary<'a>] {
        &self.import_libraries
    }

    /// Returns the virtual address execution begins at.
    #[must_use]
    pub const fn entry_point(&self) -> Option<u32> {
        self.entry_point
    }

    /// Returns the virtual address the image loads at.
    ///
    /// The security info records this too, and the two agree in practice, but
    /// this is the value the optional header states directly.
    #[must_use]
    pub const fn image_base_address(&self) -> Option<u32> {
        self.image_base_address
    }

    /// Returns the container format identified by the magic.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// Returns the raw module flags word.
    #[must_use]
    pub const fn module_flags(&self) -> u32 {
        self.module_flags
    }

    /// Returns the file offset at which the executable body begins.
    #[must_use]
    pub const fn image_offset(&self) -> u32 {
        self.image_offset
    }

    /// Returns the file offset of the security info block.
    #[must_use]
    pub const fn security_info_offset(&self) -> u32 {
        self.security_info_offset
    }

    /// Returns every optional header entry in the order the file stored them.
    #[must_use]
    pub fn optional_headers(&self) -> &[OptionalHeader<'a>] {
        &self.optional_headers
    }

    /// Returns the value of the entry with `key`, if the container carried one.
    #[must_use]
    pub fn optional_header(&self, key: OptionalHeaderKey) -> Option<OptionalHeaderValue<'a>> {
        self.optional_headers
            .iter()
            .find(|header| header.key == key)
            .map(|header| header.value)
    }
}

/// Reads the four byte magic that opens a container.
fn read_magic(reader: &mut Reader<'_>) -> Result<[u8; 4]> {
    let bytes = reader.take(4, "magic")?;
    <[u8; 4]>::try_from(bytes).map_err(|_| Error::Truncated {
        field: "magic",
        offset: 0,
        needed: 4,
        available: bytes.len(),
    })
}

/// Walks the optional header directory, resolving each entry's data.
fn parse_optional_headers<'a>(
    reader: &mut Reader<'a>,
    count: u32,
) -> Result<Vec<OptionalHeader<'a>>> {
    const ENTRY_SIZE: usize = 8;

    let declared = usize::try_from(count).unwrap_or(usize::MAX);
    let needed = declared.saturating_mul(ENTRY_SIZE);
    if needed > reader.remaining() {
        return Err(Error::Truncated {
            field: "optional_header_directory",
            offset: reader.offset(),
            needed,
            available: reader.remaining(),
        });
    }

    let mut headers = Vec::with_capacity(declared);
    for _ in 0..declared {
        let key = OptionalHeaderKey(reader.u32("optional_header_key")?);
        let value = reader.u32("optional_header_value")?;
        headers.push(OptionalHeader {
            key,
            value: resolve_value(reader, key, value)?,
        });
    }

    Ok(headers)
}

/// Resolves the data an optional header entry refers to.
fn resolve_value<'a>(
    reader: &Reader<'a>,
    key: OptionalHeaderKey,
    value: u32,
) -> Result<OptionalHeaderValue<'a>> {
    let target = u64::from(value);
    let out_of_range = || Error::OptionalHeaderOutOfRange {
        key: key.0,
        target,
        len: reader.len(),
    };

    let length = match key.storage() {
        Storage::Inline => return Ok(OptionalHeaderValue::Inline(value)),
        Storage::Fixed(words) => words.saturating_mul(4),
        Storage::Variable => {
            let mut head = reader
                .at(target, "optional_header_data")
                .map_err(|_| out_of_range())?;
            let declared = head
                .u32("optional_header_size")
                .map_err(|_| out_of_range())?;
            if declared < 4 {
                return Err(Error::OptionalHeaderInvalidSize {
                    key: key.0,
                    size: declared,
                });
            }
            declared
        }
    };

    let length = usize::try_from(length).unwrap_or(usize::MAX);
    let mut span = reader
        .at(target, "optional_header_data")
        .map_err(|_| out_of_range())?;
    let bytes = span
        .take(length, "optional_header_data")
        .map_err(|_| out_of_range())?;

    Ok(OptionalHeaderValue::Data(bytes))
}

#[cfg(test)]
pub(crate) mod build {
    //! Construction of synthetic containers for tests.
    //!
    //! Tests build containers field by field rather than loading real files,
    //! which keeps the suite free of game data and lets a test state exactly
    //! the malformed layout it is checking.

    /// Byte length of the fixed container header.
    pub(crate) const HEADER_SIZE: usize = 0x18;

    /// Byte length of the security info block before its descriptor array.
    pub(crate) const SECURITY_FIXED_SIZE: usize = 0x184;

    /// The data an entry contributes to the file.
    enum Payload {
        /// Stored in the directory entry itself.
        Inline(u32),
        /// Appended after the security info and referenced by offset.
        Trailing(Vec<u8>),
    }

    /// Builds a synthetic container byte for byte.
    pub(crate) struct ContainerBuilder {
        magic: [u8; 4],
        module_flags: u32,
        image_offset: u32,
        security_info_offset: Option<u32>,
        image_size: u32,
        load_address: u32,
        import_table_count: u32,
        export_table: u32,
        descriptors: Vec<(u32, u8)>,
        override_descriptor_count: Option<u32>,
        entries: Vec<(u32, Payload)>,
        override_count: Option<u32>,
    }

    impl ContainerBuilder {
        /// Starts a well formed XEX2 container carrying one code page.
        pub(crate) fn new() -> Self {
            Self {
                magic: *b"XEX2",
                module_flags: 0,
                image_offset: 0,
                security_info_offset: None,
                image_size: 0x1_0000,
                load_address: 0x8200_0000,
                import_table_count: 0,
                export_table: 0,
                descriptors: vec![(1, 1)],
                override_descriptor_count: None,
                entries: Vec::new(),
                override_count: None,
            }
        }

        /// Replaces the container magic.
        pub(crate) fn magic(mut self, magic: [u8; 4]) -> Self {
            self.magic = magic;
            self
        }

        /// Sets the module flags word.
        pub(crate) fn module_flags(mut self, flags: u32) -> Self {
            self.module_flags = flags;
            self
        }

        /// Sets the offset of the executable body.
        pub(crate) fn image_offset(mut self, offset: u32) -> Self {
            self.image_offset = offset;
            self
        }

        /// Points the header at a security info offset of the caller's choosing.
        pub(crate) fn security_info_offset(mut self, offset: u32) -> Self {
            self.security_info_offset = Some(offset);
            self
        }

        /// Sets the decoded image size recorded in the security info.
        pub(crate) fn image_size(mut self, size: u32) -> Self {
            self.image_size = size;
            self
        }

        /// Sets the address the image loads at.
        pub(crate) fn load_address(mut self, address: u32) -> Self {
            self.load_address = address;
            self
        }

        /// Sets the declared import table count.
        pub(crate) fn import_table_count(mut self, count: u32) -> Self {
            self.import_table_count = count;
            self
        }

        /// Sets the export table address, where zero means absent.
        pub(crate) fn export_table(mut self, address: u32) -> Self {
            self.export_table = address;
            self
        }

        /// Replaces the page descriptors, each a page count and a kind nibble.
        pub(crate) fn descriptors(mut self, descriptors: Vec<(u32, u8)>) -> Self {
            self.descriptors = descriptors;
            self
        }

        /// Writes a descriptor count that disagrees with the descriptors present.
        pub(crate) fn declared_descriptor_count(mut self, count: u32) -> Self {
            self.override_descriptor_count = Some(count);
            self
        }

        /// Adds an entry whose data is the directory value itself.
        pub(crate) fn inline(mut self, id: u32, value: u32) -> Self {
            self.entries
                .push(((id << 8) | 0x01, Payload::Inline(value)));
            self
        }

        /// Adds an entry referencing a fixed size structure of `words` words.
        ///
        /// Only meaningful for two words or more. A single dword fits in the
        /// directory value itself, which is what a low byte of one encodes.
        pub(crate) fn fixed(mut self, id: u32, words: u32, payload: Vec<u8>) -> Self {
            self.entries
                .push(((id << 8) | words, Payload::Trailing(payload)));
            self
        }

        /// Adds an entry referencing a structure that opens with its own size.
        pub(crate) fn variable(mut self, id: u32, payload: &[u8]) -> Self {
            let total = u32::try_from(payload.len() + 4).unwrap_or(u32::MAX);
            let mut bytes = total.to_be_bytes().to_vec();
            bytes.extend_from_slice(payload);
            self.entries
                .push(((id << 8) | 0xff, Payload::Trailing(bytes)));
            self
        }

        /// Adds an entry with an exact key and value, for testing bad targets.
        pub(crate) fn raw(mut self, key: u32, value: u32) -> Self {
            self.entries.push((key, Payload::Inline(value)));
            self
        }

        /// Writes a directory count that disagrees with the entries present.
        pub(crate) fn declared_count(mut self, count: u32) -> Self {
            self.override_count = Some(count);
            self
        }

        /// Serializes the security info block.
        fn security_bytes(&self) -> Vec<u8> {
            let count = self
                .override_descriptor_count
                .unwrap_or_else(|| u32::try_from(self.descriptors.len()).unwrap_or(u32::MAX));
            let header_size = u32::try_from(SECURITY_FIXED_SIZE + self.descriptors.len() * 24)
                .unwrap_or(u32::MAX);

            let mut bytes = Vec::with_capacity(SECURITY_FIXED_SIZE + self.descriptors.len() * 24);
            bytes.extend_from_slice(&header_size.to_be_bytes());
            bytes.extend_from_slice(&self.image_size.to_be_bytes());
            bytes.extend_from_slice(&[0u8; 256]);
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&self.load_address.to_be_bytes());
            bytes.extend_from_slice(&[0u8; 20]);
            bytes.extend_from_slice(&self.import_table_count.to_be_bytes());
            bytes.extend_from_slice(&[0u8; 20]);
            bytes.extend_from_slice(&[0u8; 16]);
            bytes.extend_from_slice(&[0u8; 16]);
            bytes.extend_from_slice(&self.export_table.to_be_bytes());
            bytes.extend_from_slice(&[0u8; 20]);
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&count.to_be_bytes());

            for (page_count, kind) in &self.descriptors {
                let packed = (page_count << 4) | u32::from(*kind);
                bytes.extend_from_slice(&packed.to_be_bytes());
                bytes.extend_from_slice(&[0u8; 20]);
            }
            bytes
        }

        /// Serializes the container.
        pub(crate) fn build(self) -> Vec<u8> {
            let directory_size = self.entries.len() * 8;
            let security = self.security_bytes();
            let security_offset = HEADER_SIZE + directory_size;
            let data_start = security_offset + security.len();

            let mut directory = Vec::with_capacity(directory_size);
            let mut trailing = Vec::new();

            for (key, payload) in self.entries {
                let value = match payload {
                    Payload::Inline(value) => value,
                    Payload::Trailing(bytes) => {
                        let offset = data_start + trailing.len();
                        trailing.extend_from_slice(&bytes);
                        u32::try_from(offset).unwrap_or(u32::MAX)
                    }
                };
                directory.extend_from_slice(&key.to_be_bytes());
                directory.extend_from_slice(&value.to_be_bytes());
            }

            let count = self
                .override_count
                .unwrap_or_else(|| u32::try_from(directory.len() / 8).unwrap_or(u32::MAX));
            let security_info_offset = self
                .security_info_offset
                .unwrap_or_else(|| u32::try_from(security_offset).unwrap_or(u32::MAX));

            let mut bytes = Vec::with_capacity(data_start + trailing.len());
            bytes.extend_from_slice(&self.magic);
            bytes.extend_from_slice(&self.module_flags.to_be_bytes());
            bytes.extend_from_slice(&self.image_offset.to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&security_info_offset.to_be_bytes());
            bytes.extend_from_slice(&count.to_be_bytes());
            bytes.extend_from_slice(&directory);
            bytes.extend_from_slice(&security);
            bytes.extend_from_slice(&trailing);
            bytes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build::{ContainerBuilder, HEADER_SIZE};
    use super::*;

    #[test]
    fn parses_a_minimal_header() {
        let bytes = ContainerBuilder::new()
            .module_flags(0x0000_0001)
            .image_offset(0x1000)
            .load_address(0x8200_0000)
            .image_size(0x0002_0000)
            .build();

        let container = Container::parse(&bytes).unwrap();

        assert_eq!(container.format(), Format::Xex2);
        assert_eq!(container.module_flags(), 0x0000_0001);
        assert_eq!(container.image_offset(), 0x1000);
        assert_eq!(
            container.security_info_offset(),
            u32::try_from(HEADER_SIZE).unwrap()
        );
        assert!(container.optional_headers().is_empty());

        let security = container.security_info();
        assert_eq!(security.load_address(), 0x8200_0000);
        assert_eq!(security.image_size(), 0x0002_0000);
        assert_eq!(security.export_table_address(), None);
    }

    #[test]
    fn rejects_a_security_info_offset_outside_the_file() {
        let bytes = ContainerBuilder::new()
            .security_info_offset(0xffff_0000)
            .build();

        let error = Container::parse(&bytes).unwrap_err();

        assert_eq!(
            error,
            Error::OffsetOutOfRange {
                field: "security_info_offset",
                target: 0xffff_0000,
                len: bytes.len(),
            }
        );
    }

    #[test]
    fn accepts_the_earlier_format() {
        let bytes = ContainerBuilder::new().magic(*b"XEX1").build();

        assert_eq!(Container::parse(&bytes).unwrap().format(), Format::Xex1);
    }

    #[test]
    fn rejects_unrecognized_magic_by_value() {
        let bytes = ContainerBuilder::new().magic(*b"NOPE").build();

        let error = Container::parse(&bytes).unwrap_err();

        assert_eq!(error, Error::UnsupportedMagic { magic: *b"NOPE" });
        assert!(error.to_string().contains("unsupported container magic"));
    }

    #[test]
    fn rejects_a_header_shorter_than_its_fixed_fields() {
        let bytes = ContainerBuilder::new().build();

        for length in 0..HEADER_SIZE {
            let error = Container::parse(&bytes[..length]).unwrap_err();
            assert!(
                matches!(
                    error,
                    Error::Truncated { .. } | Error::UnsupportedMagic { .. }
                ),
                "length {length} gave {error:?}"
            );
        }
    }

    #[test]
    fn resolves_an_inline_entry() {
        let bytes = ContainerBuilder::new()
            .inline(0x00_0180, 0xdead_beef)
            .build();

        let container = Container::parse(&bytes).unwrap();
        let value = container.optional_header(OptionalHeaderKey((0x00_0180 << 8) | 0x01));

        assert_eq!(value, Some(OptionalHeaderValue::Inline(0xdead_beef)));
        assert_eq!(
            value.and_then(OptionalHeaderValue::inline),
            Some(0xdead_beef)
        );
        assert_eq!(value.and_then(OptionalHeaderValue::data), None);
    }

    #[test]
    fn resolves_a_fixed_size_entry() {
        let payload = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let bytes = ContainerBuilder::new()
            .fixed(0x0002, 2, payload.clone())
            .build();

        let container = Container::parse(&bytes).unwrap();
        let value = container.optional_header(OptionalHeaderKey((0x0002 << 8) | 2));

        assert_eq!(
            value.and_then(OptionalHeaderValue::data),
            Some(payload.as_slice())
        );
    }

    #[test]
    fn resolves_a_variable_size_entry() {
        let payload = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let bytes = ContainerBuilder::new().variable(0x0003, &payload).build();

        let container = Container::parse(&bytes).unwrap();
        let data = container
            .optional_header(OptionalHeaderKey((0x0003 << 8) | 0xff))
            .and_then(OptionalHeaderValue::data)
            .unwrap();

        assert_eq!(data.len(), payload.len() + 4);
        assert_eq!(&data[4..], &payload);
    }

    #[test]
    fn retains_entries_whose_key_is_not_recognized() {
        let payload = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let bytes = ContainerBuilder::new()
            .fixed(0xbeef, 2, payload.clone())
            .build();

        let container = Container::parse(&bytes).unwrap();
        let headers = container.optional_headers();

        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].key, OptionalHeaderKey((0xbeef << 8) | 2));
        assert_eq!(headers[0].value.data(), Some(payload.as_slice()));
    }

    #[test]
    fn preserves_directory_order() {
        let bytes = ContainerBuilder::new()
            .inline(0x0001, 0x1111_1111)
            .inline(0x0002, 0x2222_2222)
            .inline(0x0003, 0x3333_3333)
            .build();

        let container = Container::parse(&bytes).unwrap();
        let keys: Vec<u32> = container
            .optional_headers()
            .iter()
            .map(|h| h.key.0)
            .collect();

        assert_eq!(
            keys,
            vec![(0x0001 << 8) | 1, (0x0002 << 8) | 1, (0x0003 << 8) | 1]
        );
    }

    #[test]
    fn rejects_an_entry_pointing_outside_the_file() {
        let key = (0x0004 << 8) | 2;
        let bytes = ContainerBuilder::new().raw(key, 0xffff_0000).build();

        let error = Container::parse(&bytes).unwrap_err();

        assert_eq!(
            error,
            Error::OptionalHeaderOutOfRange {
                key,
                target: 0xffff_0000,
                len: bytes.len()
            }
        );
        assert!(error.to_string().contains("optional header"));
    }

    #[test]
    fn rejects_an_entry_whose_data_runs_past_the_end() {
        let key = (0x0005 << 8) | 0xfe;
        let value = u32::try_from(HEADER_SIZE + 8).unwrap();
        let bytes = ContainerBuilder::new().raw(key, value).build();

        let error = Container::parse(&bytes).unwrap_err();

        assert!(
            matches!(error, Error::OptionalHeaderOutOfRange { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn rejects_a_variable_entry_declaring_an_impossible_size() {
        let key = (0x0006 << 8) | 0xff;
        let mut bytes = ContainerBuilder::new().raw(key, 0).build();
        let target = u32::try_from(bytes.len()).unwrap();
        bytes.extend_from_slice(&3u32.to_be_bytes());

        let offset = HEADER_SIZE + 4;
        bytes[offset..offset + 4].copy_from_slice(&target.to_be_bytes());

        let error = Container::parse(&bytes).unwrap_err();

        assert_eq!(error, Error::OptionalHeaderInvalidSize { key, size: 3 });
    }

    #[test]
    fn rejects_a_directory_count_larger_than_the_input() {
        let bytes = ContainerBuilder::new()
            .inline(0x0001, 0)
            .declared_count(1_000_000)
            .build();

        let error = Container::parse(&bytes).unwrap_err();

        assert!(matches!(
            error,
            Error::Truncated {
                field: "optional_header_directory",
                ..
            }
        ));
    }

    #[test]
    fn rejects_a_directory_count_at_the_u32_maximum() {
        let bytes = ContainerBuilder::new().declared_count(u32::MAX).build();

        let error = Container::parse(&bytes).unwrap_err();

        assert!(matches!(error, Error::Truncated { .. }), "{error:?}");
    }

    #[test]
    fn truncation_at_every_length_errors_without_panicking() {
        let bytes = ContainerBuilder::new()
            .inline(0x0001, 0xabcd)
            .fixed(0x0002, 1, vec![1, 2, 3, 4])
            .variable(0x0003, &[9, 9, 9, 9])
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
