//! Every exit code the diagnostics page states, produced by running the binary.
//!
//! The page and this file have to agree. A page that says a failure exits with
//! one number while the tool exits with another is worse than no page, because
//! a script written against it silently takes the wrong branch. Each test below
//! is named after the case the page describes, so a failure names the line to
//! change.

use std::process::Command;

/// The code a run exited with, or a message saying it was killed by a signal.
///
/// A helper rather than a test, so the workspace's allowance for `expect` in a
/// test does not reach it. The spawn failing is the one thing that makes this
/// file meaningless, and stopping with the arguments named is the useful answer.
#[allow(clippy::expect_used, clippy::panic)]
fn code_of(arguments: &[&str]) -> i32 {
    let output = Command::new(env!("CARGO_BIN_EXE_xenolith"))
        .args(arguments)
        .output()
        .expect("the binary should run");
    output.status.code().unwrap_or_else(|| {
        panic!("the run was ended by a signal rather than exiting: {arguments:?}")
    })
}

/// The output of a run, for checking what it said as well as what it exited
/// with.
#[allow(clippy::expect_used)]
fn said(arguments: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_xenolith"))
        .args(arguments)
        .output()
        .expect("the binary should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The page's table, read so that the test can fail when a code moves.
#[test]
fn a_command_that_ran_exits_with_nought() {
    assert_eq!(code_of(&["--help"]), 0, "--help should exit 0");
    assert_eq!(code_of(&["--version"]), 0, "--version should exit 0");
}

#[test]
fn a_command_that_could_not_run_exits_with_one() {
    assert_eq!(
        code_of(&["inspect", "/nonexistent.xex"]),
        1,
        "a file that does not exist should exit 1"
    );
    assert_eq!(
        code_of(&[
            "lift",
            "/nonexistent.xex",
            "--out",
            "/tmp/xenolith-exit-codes"
        ]),
        1,
        "lifting a file that does not exist should exit 1"
    );
}

#[test]
fn arguments_that_are_not_a_command_exit_with_two() {
    assert_eq!(code_of(&[]), 2, "no subcommand at all should exit 2");
    assert_eq!(
        code_of(&["not-a-subcommand"]),
        2,
        "a subcommand that does not exist should exit 2"
    );
}

/// A part size of zero is the case the page names, so it is the one checked.
#[test]
fn a_part_size_of_zero_exits_with_one_and_says_why() {
    let complaint = said(&[
        "lift",
        "/nonexistent.xex",
        "--out",
        "/tmp/x",
        "--part-size",
        "0",
    ]);
    assert_eq!(
        code_of(&[
            "lift",
            "/nonexistent.xex",
            "--out",
            "/tmp/x",
            "--part-size",
            "0"
        ]),
        1,
        "a part size of zero should exit 1"
    );
    assert!(
        complaint.contains("part size"),
        "the message should name the part size: {complaint}"
    );
}

#[test]
fn a_root_that_is_not_a_number_exits_with_one() {
    assert_eq!(
        code_of(&[
            "lift",
            "/nonexistent.xex",
            "--out",
            "/tmp/x",
            "--root",
            "nonsense"
        ]),
        1,
        "a root that is not a number should exit 1"
    );
}

/// A missing file says what it was doing and passes the cause through.
#[test]
fn a_missing_file_names_the_operation_and_the_cause() {
    let complaint = said(&["inspect", "/nonexistent.xex"]);
    assert!(
        complaint.contains("reading /nonexistent.xex"),
        "the message should name the file it was reading: {complaint}"
    );
    assert!(
        complaint.contains("No such file"),
        "the cause should be passed through rather than paraphrased: {complaint}"
    );
}

/// An unaligned address is refused rather than read.
#[test]
fn an_unaligned_address_is_refused_with_one() {
    assert_eq!(
        code_of(&[
            "disasm",
            "/nonexistent.xex",
            "--start",
            "0xfefefefe",
            "--length",
            "4"
        ]),
        1,
        "an address that is not four byte aligned should exit 1"
    );
}
