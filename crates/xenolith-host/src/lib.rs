//! What a program built by xenolith asks of it while a game runs.
//!
//! A built program is C, and two things it needs are written in Rust: saying a
//! shader the game loads from its data while running, and reading the game's
//! executable out of the player's own copy. Both are reached through this one
//! static library.
//!
//! Reading the executable here is what lets a built program be handed to
//! someone without the game in it. The program holds the translation and
//! nothing of the game's own bytes, and it reads those, with the player's key,
//! from the copy the player owns.

// The C face is the whole point of this crate, and it cannot be written
// without handing pointers across. Every unsafe block says what it relies on.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char};
use std::path::{Path, PathBuf};

use xenolith_xex::{Container, EncryptionType, Image, KeyMaterial};

pub use xenolith_shader::running::xenolith_say_a_shader;

/// Environment variable holding the key as hexadecimal, as the tool reads it.
const KEY_ENV: &str = "XENOLITH_XEX_KEY";

/// Where the key file is looked for, as the tool looks for it.
fn key_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("xenolith")
        .join("xex.key")
}

/// Finds the player's key: the environment first, then the key file.
fn find_the_key() -> Result<Option<KeyMaterial>, String> {
    if let Some(text) = std::env::var_os(KEY_ENV) {
        let text = text
            .into_string()
            .map_err(|_| format!("{KEY_ENV} is not valid text"))?;
        return KeyMaterial::from_hex(&text)
            .map(Some)
            .map_err(|why| format!("{KEY_ENV} is not a key: {why}"));
    }
    let path = key_path();
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|why| format!("{} could not be read: {why}", path.display()))?;
    KeyMaterial::from_hex(text.trim())
        .map(Some)
        .map_err(|why| format!("{} is not a key: {why}", path.display()))
}

/// Reads a game's executable and decodes its image, with the player's key
/// where it is encrypted.
///
/// # Errors
///
/// Returns why, in words for the player, when the file cannot be read, is not
/// an executable, is encrypted with no key to hand, or does not decode.
pub fn read_the_game(path: &Path) -> Result<Image, String> {
    let bytes = std::fs::read(path)
        .map_err(|why| format!("{} could not be read: {why}", path.display()))?;
    let container = Container::parse(&bytes)
        .map_err(|why| format!("{} is not a game's executable: {why}", path.display()))?;
    let key = find_the_key()?;
    if container.encryption() == EncryptionType::Encrypted && key.is_none() {
        return Err(format!(
            "{} is encrypted and no key was found: set {KEY_ENV} to the key as 32 hexadecimal \
             digits, or write it to {}",
            path.display(),
            key_path().display()
        ));
    }
    container
        .load(key.as_ref())
        .map_err(|why| format!("{} would not decode: {why}", path.display()))
}

/// Reads a game's executable into memory a built program has ready for it.
///
/// Returns how many bytes were written, and says the image's fingerprint, so
/// the program can check it is the one it was built from. Returns nought where
/// the image could not be read or is larger than `room`, with why written into
/// `why` as text ending in a nought byte.
///
/// # Safety
///
/// `path` must point at text ending in a nought byte. `into` must point at
/// `room` writable bytes, `fingerprint` at one writable word, and `why` at
/// `why_room` writable bytes. `why` may be null where `why_room` is nought.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xenolith_read_the_game(
    path: *const c_char,
    into: *mut u8,
    room: usize,
    fingerprint: *mut u64,
    why: *mut c_char,
    why_room: usize,
) -> usize {
    let say = |text: &str| {
        if why.is_null() || why_room == 0 {
            return;
        }
        let length = text.len().min(why_room - 1);
        // SAFETY: the caller says `why` has `why_room` bytes, and this writes
        // at most one fewer and then the nought after them.
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr().cast::<c_char>(), why, length);
            *why.add(length) = 0;
        }
    };
    if path.is_null() || into.is_null() || fingerprint.is_null() {
        say("nothing was named to read");
        return 0;
    }

    // SAFETY: the caller says `path` ends in a nought byte.
    let named = unsafe { CStr::from_ptr(path) };
    let Ok(named) = named.to_str() else {
        say("the game's path is not valid text");
        return 0;
    };
    let image = match read_the_game(Path::new(named)) {
        Ok(image) => image,
        Err(text) => {
            say(&text);
            return 0;
        }
    };
    let bytes = image.bytes();
    if bytes.len() > room {
        say("the game's image is larger than the room made for it");
        return 0;
    }

    // SAFETY: the caller says `into` has `room` bytes, and the line above has
    // established the image is no longer than that. `fingerprint` is one word.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), into, bytes.len());
        *fingerprint = image.fingerprint();
    }
    bytes.len()
}
