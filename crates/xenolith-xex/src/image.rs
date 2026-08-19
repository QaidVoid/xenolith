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
