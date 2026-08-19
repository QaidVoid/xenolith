//! Decryption of the image body.
//!
//! An encrypted container seals its image with a session key, and stores that
//! session key wrapped under a static key. Decoding therefore takes two steps:
//! unwrap the session key, then decrypt the body with it. Both use AES-128 in
//! CBC mode with a zero initialization vector.
//!
//! No static key is compiled into this crate. The caller supplies it, which
//! keeps key material out of the source tree and makes the dependency explicit
//! at the point of use.

use aes::Aes128;
use aes::cipher::block_padding::NoPadding;
use aes::cipher::{BlockModeDecrypt, KeyIvInit};

use crate::error::{Error, Result};

/// AES-128-CBC decryptor.
type Decryptor = cbc::Decryptor<Aes128>;

/// Bytes in an AES block, and therefore in a session key.
pub(crate) const BLOCK_SIZE: usize = 16;

/// The initialization vector both decryption steps use.
const ZERO_IV: [u8; BLOCK_SIZE] = [0; BLOCK_SIZE];

/// A static key supplied by the caller, used to unwrap a session key.
///
/// Its [`core::fmt::Debug`] rendering is redacted, so key bytes cannot reach a
/// log through an incidental `{:?}`.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyMaterial([u8; BLOCK_SIZE]);

impl KeyMaterial {
    /// Wraps a 16 byte key.
    #[must_use]
    pub const fn new(key: [u8; BLOCK_SIZE]) -> Self {
        Self(key)
    }

    /// Parses a key from 32 hexadecimal characters.
    ///
    /// Underscores and whitespace are ignored so that keys can be written in
    /// whatever grouping the caller finds readable.
    ///
    /// # Errors
    ///
    /// Returns an error when the input does not hold exactly 32 hexadecimal
    /// digits.
    pub fn from_hex(text: &str) -> Result<Self> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'_')
            .collect();

        if digits.len() != BLOCK_SIZE * 2 {
            return Err(Error::KeyMaterialMalformed {
                digits: digits.len(),
            });
        }

        let mut key = [0u8; BLOCK_SIZE];
        for (index, pair) in digits.chunks_exact(2).enumerate() {
            let high = hex_value(pair.first().copied().unwrap_or(0))?;
            let low = hex_value(pair.get(1).copied().unwrap_or(0))?;
            if let Some(slot) = key.get_mut(index) {
                *slot = (high << 4) | low;
            }
        }

        Ok(Self(key))
    }
}

impl core::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("KeyMaterial(redacted)")
    }
}

/// Converts one hexadecimal digit to its value.
fn hex_value(digit: u8) -> Result<u8> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        b'A'..=b'F' => Ok(digit - b'A' + 10),
        other => Err(Error::KeyMaterialInvalidDigit {
            digit: char::from(other),
        }),
    }
}

/// Unwraps the session key stored in the security info.
///
/// The input is exactly one cipher block, which cannot fail to unpad under
/// [`NoPadding`]. The unreachable error path yields the input unchanged rather
/// than panicking, since a panic here would be a worse outcome than a wrong key
/// that the image validation downstream will reject anyway.
pub(crate) fn unwrap_session_key(
    wrapped: &[u8; BLOCK_SIZE],
    key: &KeyMaterial,
) -> [u8; BLOCK_SIZE] {
    let mut session = *wrapped;
    let decryptor = Decryptor::new(&key.0.into(), &ZERO_IV.into());

    match decryptor.decrypt_padded::<NoPadding>(&mut session) {
        Ok(_) => session,
        Err(_) => *wrapped,
    }
}

/// Decrypts an encrypted image body with an unwrapped session key.
///
/// # Errors
///
/// Returns an error when the body is not a whole number of AES blocks.
pub(crate) fn decrypt_image(body: &[u8], session_key: &[u8; BLOCK_SIZE]) -> Result<Vec<u8>> {
    if body.len() % BLOCK_SIZE != 0 {
        return Err(Error::ImageNotBlockAligned { len: body.len() });
    }

    let mut plaintext = body.to_vec();
    let decryptor = Decryptor::new(session_key.into(), &ZERO_IV.into());
    decryptor
        .decrypt_padded::<NoPadding>(&mut plaintext)
        .map_err(|_| Error::ImageNotBlockAligned { len: body.len() })?;

    Ok(plaintext)
}

/// Encrypts with the same parameters the decoder expects, for test fixtures.
#[cfg(test)]
pub(crate) fn encrypt(plaintext: &[u8], key: &[u8; BLOCK_SIZE]) -> Vec<u8> {
    use aes::cipher::BlockModeEncrypt;

    let mut buffer = plaintext.to_vec();
    let length = buffer.len();
    cbc::Encryptor::<Aes128>::new(key.into(), &ZERO_IV.into())
        .encrypt_padded::<NoPadding>(&mut buffer, length)
        .expect("fixture encryption should not fail");
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_hex_key() {
        let key = KeyMaterial::from_hex("000102030405060708090a0b0c0d0e0f").unwrap();

        assert_eq!(
            key,
            KeyMaterial::new([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
        );
    }

    #[test]
    fn ignores_grouping_in_a_hex_key() {
        let spaced = KeyMaterial::from_hex("00010203_04050607 08090a0b_0c0d0e0f").unwrap();
        let plain = KeyMaterial::from_hex("000102030405060708090a0b0c0d0e0f").unwrap();

        assert_eq!(spaced, plain);
    }

    #[test]
    fn rejects_a_hex_key_of_the_wrong_length() {
        assert!(KeyMaterial::from_hex("00010203").is_err());
        assert!(KeyMaterial::from_hex(&"a".repeat(33)).is_err());
    }

    #[test]
    fn rejects_a_hex_key_with_non_hex_digits() {
        assert!(KeyMaterial::from_hex("zz0102030405060708090a0b0c0d0e0f").is_err());
    }

    #[test]
    fn debug_does_not_leak_the_key() {
        let key = KeyMaterial::new([0xab; BLOCK_SIZE]);

        assert_eq!(format!("{key:?}"), "KeyMaterial(redacted)");
        assert!(!format!("{key:?}").contains("ab"));
    }

    #[test]
    fn unwraps_a_session_key_that_was_wrapped_with_the_same_static_key() {
        let static_key = KeyMaterial::new([0x11; BLOCK_SIZE]);
        let session = [0x42u8; BLOCK_SIZE];
        let wrapped: [u8; BLOCK_SIZE] = encrypt(&session, &static_key.0).try_into().unwrap();

        assert_eq!(unwrap_session_key(&wrapped, &static_key), session);
    }

    #[test]
    fn a_different_static_key_yields_a_different_session_key() {
        let right = KeyMaterial::new([0x11; BLOCK_SIZE]);
        let wrong = KeyMaterial::new([0x22; BLOCK_SIZE]);
        let session = [0x42u8; BLOCK_SIZE];
        let wrapped: [u8; BLOCK_SIZE] = encrypt(&session, &right.0).try_into().unwrap();

        assert_ne!(unwrap_session_key(&wrapped, &wrong), session);
    }

    #[test]
    fn decrypts_an_image_round_trip() {
        let session = [0x7fu8; BLOCK_SIZE];
        let plaintext: Vec<u8> = (0..64u8).collect();
        let ciphertext = encrypt(&plaintext, &session);

        assert_eq!(decrypt_image(&ciphertext, &session).unwrap(), plaintext);
    }

    #[test]
    fn decrypts_an_empty_body() {
        let session = [0x7fu8; BLOCK_SIZE];

        assert_eq!(decrypt_image(&[], &session).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn rejects_a_body_that_is_not_whole_blocks() {
        let session = [0x7fu8; BLOCK_SIZE];

        let error = decrypt_image(&[0u8; 20], &session).unwrap_err();

        assert_eq!(error, Error::ImageNotBlockAligned { len: 20 });
    }

    #[test]
    fn a_wrong_session_key_does_not_recover_the_plaintext() {
        let right = [0x7fu8; BLOCK_SIZE];
        let wrong = [0x80u8; BLOCK_SIZE];
        let plaintext: Vec<u8> = (0..64u8).collect();
        let ciphertext = encrypt(&plaintext, &right);

        assert_ne!(decrypt_image(&ciphertext, &wrong).unwrap(), plaintext);
    }
}
