//! The C face of this crate, which is the only reason it is built as a library.
//!
//! A built program is C, and it reaches this crate for two things: saying a
//! shader a game loads while running, and reading the game's executable out of
//! the player's copy. Both are only reachable if the crate is built as a static
//! library, and a manifest rewritten without that builds every other package
//! just as well and says nothing. This looks.

// Calling the C face the way C does is the only way to check what it hands
// back, and that takes pointers.
#![allow(unsafe_code)]

use std::path::Path;

/// The kinds of library the manifest asks for.
///
/// # Panics
///
/// Where the manifest cannot be read or declares no kinds, because a check that
/// cannot see them must not pass by finding nothing.
#[allow(clippy::panic)]
fn crate_types() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&path)
        .unwrap_or_else(|why| panic!("cannot read {}: {why}", path.display()));
    let line = manifest
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("crate-type"))
        .unwrap_or_else(|| panic!("the manifest declares no crate-type at all"));
    let Some((_, list)) = line.split_once('=') else {
        panic!("the crate-type line is not an assignment: {line}");
    };
    list.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|one| one.trim().trim_matches('"').to_owned())
        .filter(|one| !one.is_empty())
        .collect()
}

#[test]
fn the_crate_is_built_as_a_static_library() {
    let kinds = crate_types();
    assert!(
        kinds.iter().any(|one| one == "staticlib"),
        "the manifest asks for {kinds:?} and not a staticlib, so a built program cannot link it"
    );
}

#[test]
fn the_functions_a_c_caller_reaches_have_c_signatures() {
    // Naming each through a C function pointer type is what fails to compile
    // where a signature drifts. The C side declares them by hand and shares
    // nothing with this crate that would catch it.
    let saying: unsafe extern "C" fn(u32, *const u32, u32, *mut u32, u32) -> u32 =
        xenolith_host::xenolith_say_a_shader;
    let reading: unsafe extern "C" fn(
        *const std::ffi::c_char,
        *mut u8,
        usize,
        *mut u64,
        *mut std::ffi::c_char,
        usize,
    ) -> usize = xenolith_host::xenolith_read_the_game;
    let _ = (saying, reading);
}

#[test]
fn a_game_that_is_not_there_is_named() {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-game-here/default.xex");
    let said = xenolith_host::read_the_game(&path)
        .err()
        .unwrap_or_default();
    assert!(
        said.contains("no-game-here/default.xex"),
        "a missing executable was not named: {said:?}"
    );
}

#[test]
fn something_that_is_not_an_executable_says_so() {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("not-a-game.xex");
    std::fs::write(&path, b"not a container").ok();
    let said = xenolith_host::read_the_game(&path)
        .err()
        .unwrap_or_default();
    assert!(
        said.contains("is not a game's executable"),
        "a file that is not an executable was not refused as one: {said:?}"
    );
}

#[test]
fn why_a_read_failed_reaches_the_c_caller() {
    let path = std::ffi::CString::new("/nowhere/default.xex").unwrap_or_default();
    let mut room = [0u8; 16];
    let mut fingerprint = 0u64;
    let mut why = [0 as std::ffi::c_char; 256];
    // SAFETY: every pointer is to a buffer of the size passed beside it.
    let read = unsafe {
        xenolith_host::xenolith_read_the_game(
            path.as_ptr(),
            room.as_mut_ptr(),
            room.len(),
            &raw mut fingerprint,
            why.as_mut_ptr(),
            why.len(),
        )
    };
    // SAFETY: the function always ends what it writes with a nought byte.
    let said = unsafe { std::ffi::CStr::from_ptr(why.as_ptr()) }.to_string_lossy();
    assert_eq!(read, 0);
    assert!(
        said.contains("/nowhere/default.xex"),
        "the reason did not arrive: {said:?}"
    );
}
