//! The C face of this crate, which is the only reason it is built as a library.
//!
//! A title hands a shader over while it is running, and the program that is
//! running is C. So this crate has one function a C caller reaches, and it is
//! only reachable at all if the crate is built as a static library: a Rust
//! library alone cannot be linked from C.
//!
//! That was lost once. The manifest was rewritten without the crate type and
//! nothing failed, because nothing in the workspace depends on this crate and a
//! build of another package says nothing about it. The archive simply stopped
//! being produced, and the only way to notice was to go and look. This looks.

use std::path::Path;

/// The manifest, read as text.
///
/// # Panics
///
/// Where the manifest cannot be read, because a check that cannot see it must
/// not pass by finding nothing.
#[allow(clippy::panic)]
fn manifest() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|why| panic!("cannot read {}: {why}", path.display()))
}

/// The kinds of library the manifest asks for, as the array it was written as.
///
/// # Panics
///
/// Where the manifest holds no `crate-type` line at all, which is a different
/// failure from one holding the wrong kinds and is worth naming separately.
#[allow(clippy::panic)]
fn crate_types() -> Vec<String> {
    let manifest = manifest();
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
        "the manifest asks for {kinds:?} and not a staticlib, so the C ABI in \
         running.rs cannot be linked from C"
    );
}

#[test]
fn the_crate_is_still_usable_as_a_rust_library() {
    let kinds = crate_types();
    assert!(
        kinds.iter().any(|one| one == "rlib"),
        "the manifest asks for {kinds:?} and not an rlib, so the tests and any \
         Rust caller could not link it"
    );
}

#[test]
fn the_function_a_c_caller_reaches_has_a_c_signature() {
    // Naming the function through a C function pointer type is what fails to
    // compile where the signature drifts, which is the thing worth catching:
    // the C side reads the answer back by hand and shares no header.
    let reachable: unsafe extern "C" fn(u32, *const u32, u32, *mut u32, u32) -> u32 =
        xenolith_shader::running::xenolith_say_a_shader;
    let _ = reachable;
}
