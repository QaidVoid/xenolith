//! Integration tests driving the built binary.

use std::process::Command;

/// Runs the tool with the given arguments.
///
/// Expands to a tuple of success, stdout, and stderr. This is a macro rather
/// than a function so the fallible spawn is expanded inside the test that calls
/// it, where a panic on failure is the appropriate response.
macro_rules! run {
    ($($arg:expr),* $(,)?) => {{
        let args: Vec<&str> = vec![$($arg),*];
        let output = Command::new(env!("CARGO_BIN_EXE_xenolith"))
            .args(&args)
            .output()
            .expect("the binary should run");

        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }};
}

#[test]
fn top_level_help_lists_the_subcommands() {
    let (ok, stdout, _) = run!("--help");

    assert!(ok, "help should succeed");
    assert!(stdout.contains("inspect"), "help did not list inspect");
}

#[test]
fn inspect_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("inspect", "--help");

    assert!(ok, "inspect help should succeed");
    assert!(stdout.contains("--key-file"), "key file not documented");
    assert!(stdout.contains("--decode"), "decode not documented");
}

#[test]
fn reports_the_version() {
    let (ok, stdout, _) = run!("--version");

    assert!(ok, "version should succeed");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_missing_input_path_fails_and_names_the_path() {
    let (ok, _, stderr) = run!("inspect", "/nonexistent/path/to/default.xex");

    assert!(!ok, "a missing file should fail");
    assert!(
        stderr.contains("/nonexistent/path/to/default.xex"),
        "the error did not name the path: {stderr}"
    );
}

#[test]
fn a_file_that_is_not_a_container_fails_with_both_the_path_and_the_reason() {
    let path = std::env::temp_dir().join("xenolith-not-a-xex.bin");
    std::fs::write(&path, b"this is definitely not a xex file").unwrap();

    let (ok, _, stderr) = run!("inspect", path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "a non container should fail");
    assert!(
        stderr.contains("xenolith-not-a-xex.bin"),
        "the error did not name the file: {stderr}"
    );
    assert!(
        stderr.contains("magic"),
        "the error did not give the reason: {stderr}"
    );
}

#[test]
fn an_empty_file_is_rejected_without_panicking() {
    let path = std::env::temp_dir().join("xenolith-empty.bin");
    std::fs::write(&path, b"").unwrap();

    let (ok, _, stderr) = run!("inspect", path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "an empty file should fail");
    assert!(!stderr.contains("panicked"), "the tool panicked: {stderr}");
}

#[test]
fn no_arguments_fails_with_usage() {
    let (ok, _, stderr) = run!();

    assert!(!ok, "no arguments should fail");
    assert!(stderr.contains("Usage"), "no usage shown: {stderr}");
}
