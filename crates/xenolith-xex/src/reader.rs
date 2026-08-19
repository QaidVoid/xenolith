//! Bounds checked cursor for reading big-endian values out of a container.
//!
//! The Xbox 360 is a big-endian machine and its container formats follow, so
//! every multi-byte field in a XEX is stored most significant byte first.
//!
//! Offsets that come out of the file are untrusted and are frequently wider
//! than a host pointer, so [`Reader::at`] takes a [`u64`] and validates the
//! conversion. Reading past the end of the input is an error rather than a
//! panic, which is what lets the parser stay total over arbitrary input.

use crate::error::{Error, Result};

/// A position within a byte slice, supporting checked big-endian reads.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    /// The complete input being read, never re-sliced so that offsets stay
    /// absolute and therefore meaningful in an error.
    bytes: &'a [u8],
    /// Byte offset the next read starts at. Never exceeds `bytes.len()`.
    offset: usize,
}

impl<'a> Reader<'a> {
    /// Creates a reader positioned at the start of `bytes`.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    /// Returns the absolute offset the next read will start at.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the total length of the input.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns how many bytes lie between the cursor and the end of the input.
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    /// Returns an independent reader positioned at an absolute file offset.
    ///
    /// The optional header directory stores offsets to structures scattered
    /// through the file, so chasing one must not disturb the cursor walking the
    /// directory itself.
    ///
    /// Fails when `target` does not fit in a host pointer or lies beyond the
    /// end of the input, naming `field` so the offending header is identifiable.
    pub(crate) fn at(&self, target: u64, field: &'static str) -> Result<Self> {
        Ok(Self {
            bytes: self.bytes,
            offset: self.checked_offset(target, field)?,
        })
    }

    /// Advances the cursor by `count` bytes.
    pub(crate) fn skip(&mut self, count: usize, field: &'static str) -> Result<()> {
        self.take(count, field).map(|_| ())
    }

    /// Reads `count` bytes and advances past them.
    pub(crate) fn take(&mut self, count: usize, field: &'static str) -> Result<&'a [u8]> {
        let start = self.offset;
        let end = start.checked_add(count).ok_or(Error::Truncated {
            field,
            offset: start,
            needed: count,
            available: self.remaining(),
        })?;

        let slice = self.bytes.get(start..end).ok_or(Error::Truncated {
            field,
            offset: start,
            needed: count,
            available: self.remaining(),
        })?;

        self.offset = end;
        Ok(slice)
    }

    /// Reads a big-endian 32-bit value.
    pub(crate) fn u32(&mut self, field: &'static str) -> Result<u32> {
        self.take_array::<4>(field).map(u32::from_be_bytes)
    }

    /// Reads a fixed size array and advances past it.
    fn take_array<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N]> {
        let start = self.offset;
        let slice = self.take(N, field)?;

        <[u8; N]>::try_from(slice).map_err(|_| Error::Truncated {
            field,
            offset: start,
            needed: N,
            available: slice.len(),
        })
    }

    /// Validates a file-supplied offset against the bounds of the input.
    fn checked_offset(&self, target: u64, field: &'static str) -> Result<usize> {
        let len = self.bytes.len();
        let out_of_range = || Error::OffsetOutOfRange { field, target, len };

        let target = usize::try_from(target).map_err(|_| out_of_range())?;
        if target > len {
            return Err(out_of_range());
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &[u8] = &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

    #[test]
    fn reads_big_endian_words() {
        let mut reader = Reader::new(INPUT);

        assert_eq!(reader.u32("first").unwrap(), 0x0102_0304);
        assert_eq!(reader.u32("second").unwrap(), 0x0506_0708);
        assert_eq!(reader.offset(), 8);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn exact_fit_read_succeeds() {
        let mut reader = Reader::new(&INPUT[..4]);

        assert_eq!(reader.u32("whole").unwrap(), 0x0102_0304);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn read_one_byte_short_fails() {
        let mut reader = Reader::new(&INPUT[..3]);

        let error = reader.u32("whole").unwrap_err();

        assert_eq!(
            error,
            Error::Truncated {
                field: "whole",
                offset: 0,
                needed: 4,
                available: 3,
            }
        );
    }

    #[test]
    fn failed_read_leaves_cursor_untouched() {
        let mut reader = Reader::new(&INPUT[..6]);
        reader.skip(4, "prefix").unwrap();

        assert!(reader.u32("too_wide").is_err());
        assert_eq!(reader.offset(), 4);
        assert_eq!(reader.take(2, "narrow").unwrap(), &[0x05, 0x06]);
    }

    #[test]
    fn read_at_end_of_input_fails() {
        let mut reader = Reader::new(INPUT);
        reader.skip(8, "all").unwrap();

        assert!(reader.take(1, "past_end").is_err());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn take_count_overflowing_pointer_width_fails() {
        let mut reader = Reader::new(INPUT);
        reader.skip(4, "prefix").unwrap();

        let error = reader.take(usize::MAX, "huge").unwrap_err();

        assert_eq!(
            error,
            Error::Truncated {
                field: "huge",
                offset: 4,
                needed: usize::MAX,
                available: 4,
            }
        );
    }

    #[test]
    fn seeking_to_the_end_is_allowed_but_reading_is_not() {
        let reader = Reader::new(INPUT);

        let mut end = reader.at(8, "end").unwrap();

        assert_eq!(end.remaining(), 0);
        assert!(end.take(1, "past_end").is_err());
    }

    #[test]
    fn seeking_past_the_end_fails() {
        let reader = Reader::new(INPUT);

        let error = reader.at(9, "directory").unwrap_err();

        assert_eq!(
            error,
            Error::OffsetOutOfRange {
                field: "directory",
                target: 9,
                len: 8,
            }
        );
    }

    #[test]
    fn seeking_beyond_pointer_width_fails_without_overflow() {
        let reader = Reader::new(INPUT);

        let error = reader.at(u64::MAX, "directory").unwrap_err();

        assert_eq!(
            error,
            Error::OffsetOutOfRange {
                field: "directory",
                target: u64::MAX,
                len: 8,
            }
        );
    }

    #[test]
    fn offsets_at_the_u32_boundary_do_not_wrap() {
        let reader = Reader::new(INPUT);

        for target in [
            u64::from(u32::MAX) - 1,
            u64::from(u32::MAX),
            u64::from(u32::MAX) + 1,
        ] {
            let error = reader.at(target, "header").unwrap_err();
            assert_eq!(
                error,
                Error::OffsetOutOfRange {
                    field: "header",
                    target,
                    len: 8,
                }
            );
        }
    }

    #[test]
    fn chasing_an_offset_leaves_the_original_cursor_alone() {
        let mut reader = Reader::new(INPUT);
        reader.skip(2, "prefix").unwrap();

        let mut chased = reader.at(4, "target").unwrap();

        assert_eq!(chased.u32("value").unwrap(), 0x0506_0708);
        assert_eq!(reader.offset(), 2);
        assert_eq!(reader.take(1, "next").unwrap(), &[0x03]);
    }

    #[test]
    fn chasing_an_out_of_range_offset_fails() {
        let reader = Reader::new(INPUT);

        assert!(reader.at(100, "target").is_err());
    }

    #[test]
    fn empty_input_reads_fail() {
        let mut reader = Reader::new(&[]);

        assert_eq!(reader.len(), 0);
        assert_eq!(reader.remaining(), 0);
        assert!(reader.take(1, "anything").is_err());
        assert!(reader.take(0, "nothing").is_ok());
    }
}
