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
