//! Error type shared by every stage of container parsing and decoding.
//!
//! A XEX file is arbitrary input chosen by whoever runs the tool, so every
//! failure has to arrive as a value rather than a panic. Each variant carries
//! the offset it failed at and the name of the field it was reading, which is
//! what makes a rejection actionable against a real file.

/// Result of a fallible container operation.
pub type Result<T> = core::result::Result<T, Error>;

/// A failure encountered while parsing or decoding a XEX container.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A read required more bytes than the input had left.
    #[error(
        "truncated input reading {field}: need {needed} bytes at offset {offset:#x}, {available} available"
    )]
    Truncated {
        /// Name of the field being read when the input ran out.
        field: &'static str,
        /// Offset the read started at.
        offset: usize,
        /// Number of bytes the read required.
        needed: usize,
        /// Number of bytes actually remaining from `offset`.
        available: usize,
    },

    /// An offset stored inside the file points outside the file.
    #[error("{field} points to offset {target:#x}, outside the {len} byte input")]
    OffsetOutOfRange {
        /// Name of the field holding the offending offset.
        field: &'static str,
        /// The offset value that fell outside the input.
        target: u64,
        /// Total length of the input.
        len: usize,
    },

    /// The file does not begin with a magic this crate recognizes.
    #[error("unsupported container magic {magic:02x?}")]
    UnsupportedMagic {
        /// The four bytes found at the start of the input.
        magic: [u8; 4],
    },

    /// An optional header entry points to data outside the file.
    #[error(
        "optional header {key:#010x} points to offset {target:#x}, outside the {len} byte input"
    )]
    OptionalHeaderOutOfRange {
        /// Key of the entry holding the offending offset.
        key: u32,
        /// The offset value that fell outside the input.
        target: u64,
        /// Total length of the input.
        len: usize,
    },

    /// The container is encrypted but no key material was supplied.
    ///
    /// This crate compiles in no key constants, so decoding an encrypted image
    /// always depends on the caller providing one.
    #[error("the image is encrypted and no key material was supplied")]
    KeyMaterialRequired,

    /// The container declares an encryption scheme this crate cannot apply.
    #[error("unsupported encryption scheme {value}")]
    UnsupportedEncryption {
        /// The scheme value that was not recognized.
        value: u16,
    },

    /// Key material was supplied but did not hold 32 hexadecimal digits.
    #[error("key material must hold 32 hexadecimal digits, found {digits}")]
    KeyMaterialMalformed {
        /// How many digits were actually present.
        digits: usize,
    },

    /// Key material contained a character that is not a hexadecimal digit.
    #[error("key material contains {digit:?}, which is not a hexadecimal digit")]
    KeyMaterialInvalidDigit {
        /// The offending character.
        digit: char,
    },

    /// An encrypted image body was not a whole number of cipher blocks.
    #[error("encrypted image body of {len} bytes is not a multiple of the 16 byte block size")]
    ImageNotBlockAligned {
        /// Length of the body that was rejected.
        len: usize,
    },

    /// The container uses a compression scheme this crate does not reconstruct.
    #[error("unsupported compression scheme {scheme}")]
    UnsupportedCompression {
        /// Name of the scheme that is not yet reconstructed.
        scheme: String,
    },

    /// A read fell outside the mapped image.
    #[error("read of {len} bytes at {address:#010x} is not mapped")]
    UnmappedRead {
        /// Address the read started at.
        address: u32,
        /// Length the read asked for.
        len: usize,
    },

    /// The container declares an image larger than the console could load.
    #[error("declared image size of {size} bytes exceeds what the console can hold")]
    ImageTooLarge {
        /// The size value that was rejected.
        size: u32,
    },

    /// The basic scheme blocks describe more bytes than the image can hold.
    #[error("basic blocks describe {described} bytes for an image of {image_size}")]
    BasicBlocksExceedImage {
        /// Total bytes the blocks account for.
        described: u64,
        /// The declared image size.
        image_size: u32,
    },

    /// The basic scheme blocks read more bytes than the body holds.
    #[error("basic blocks read {stored} stored bytes, but the body holds {available}")]
    BasicBlocksExceedBody {
        /// Total stored bytes the blocks claim.
        stored: u64,
        /// Bytes actually present in the body.
        available: usize,
    },

    /// The file format info declared a size that does not describe whole blocks.
    #[error("file format info declares an invalid size of {size} bytes")]
    FileFormatInfoInvalidSize {
        /// The size value that was rejected.
        size: u32,
    },

    /// An import library record's declared size disagrees with its contents.
    ///
    /// A record is a fixed header followed by one address per import, so the
    /// size is fully determined by the import count. A disagreement means the
    /// record was misread and walking to the next one would compound the error.
    #[error("import library at offset {offset:#x} declares an inconsistent size of {size} bytes")]
    ImportLibraryInvalidSize {
        /// Offset of the record within the payload.
        offset: usize,
        /// The size the record declared.
        size: u32,
    },

    /// An import library named a string table entry that does not exist.
    #[error("import library names entry {index}, but the string table holds {count}")]
    ImportLibraryNameIndex {
        /// The index the record held.
        index: u16,
        /// How many names the string table actually held.
        count: usize,
    },

    /// A variable length optional header declared a size that cannot be valid.
    ///
    /// The declared size covers the length field itself, so anything below four
    /// bytes describes a structure smaller than its own header.
    #[error("optional header {key:#010x} declares an invalid size of {size} bytes")]
    OptionalHeaderInvalidSize {
        /// Key of the entry holding the offending size.
        key: u32,
        /// The size value that was rejected.
        size: u32,
    },
}
