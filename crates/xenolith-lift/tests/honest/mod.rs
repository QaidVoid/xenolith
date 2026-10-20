//! Running a check that needs something the machine may not have.
//!
//! Two kinds of thing are needed and they are not the same kind.
//!
//! A host C compiler is needed by most of this suite. It is not game data and it
//! is not a cross toolchain: it is an ordinary program that a machine building
//! this workspace already has, and a suite that quietly skipped every test
//! needing one would report green with the strongest checks dark. So a missing
//! compiler panics and names what to install.
//!
//! Game data, a cross toolchain and an emulator are different. They cannot be
//! required of everyone, because they are large, licensed, or simply not present
//! on a machine that is otherwise able to build and check this project. A test
//! needing one is marked `#[ignore]`, so a default run counts it as ignored
//! rather than passed, and when it is included without its input it fails and
//! names the variable that would have run it. An ignored test is its own guard:
//! there is no separate test that checks the first one was skipped.
//!
//! In a subdirectory because a file directly under `tests/` is a test target of
//! its own, which would compile this module a second time with nothing using
//! it.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of this, and the module is shared"
)]
// This module is compiled into a test binary, where panicking is how a check
// reports what it found, but clippy only knows that for the test functions
// themselves and not for a module they call into.
#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "a check that could not run must fail, and this module exists to make it"
)]

use std::path::PathBuf;
use std::process::Command;

/// Returns a host C compiler, or panics naming what to install.
///
/// Every test that needs one needs it for the same reason: the emitted C is only
/// checked once something has accepted it. A suite that skipped those would be
/// checking strings.
#[must_use]
pub(crate) fn host_compiler() -> &'static str {
    ["clang", "cc", "gcc"]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .expect("a host C compiler is required to check the emitted code; install clang or gcc")
}

/// Returns the value of an environment variable, or fails naming it.
///
/// The message names the variable, because a run that was included and could not
/// find its input has to say which input to supply rather than reporting that
/// nothing went wrong.
pub(crate) fn require(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set, so this check could not run"))
}

/// Returns the value of an environment variable as a path, or fails naming it.
pub(crate) fn require_path(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is not set, so this check could not run"))
}

/// Returns what a check produced, or fails the test with the reason it gave.
///
/// A check that ran and disagreed panics with the disagreement, which is the
/// thing a reader is looking for. A check that could not run has already failed
/// through [`require`] and never reaches here.
#[must_use]
pub(crate) fn or_fail<T>(outcome: Result<T, String>, what: &str) -> T {
    match outcome {
        Ok(thing) => thing,
        Err(complaint) => panic!("{what}: {complaint}"),
    }
}

/// Builds a directory to work in, made fresh so a stale binary cannot be run.
pub(crate) fn workspace(name: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a directory to build in");
    directory
}
