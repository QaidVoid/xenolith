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

#[test]
fn disasm_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("disasm", "--help");

    assert!(ok, "disasm help should succeed");
    for flag in ["--raw", "--base", "--start", "--length", "--end", "--sweep"] {
        assert!(stdout.contains(flag), "{flag} not documented");
    }
}

#[test]
fn a_misaligned_start_is_refused() {
    let path = std::env::temp_dir().join("xenolith-align.bin");
    std::fs::write(&path, vec![0x60u8; 64]).unwrap();

    let (ok, _, stderr) = run!(
        "disasm",
        "--raw",
        "--base",
        "0x82000000",
        "--start",
        "0x82000002",
        path.to_str().unwrap()
    );
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "a misaligned start should fail");
    assert!(
        stderr.contains("aligned"),
        "the error did not explain the alignment rule: {stderr}"
    );
}

#[test]
fn a_range_outside_the_image_is_refused() {
    let path = std::env::temp_dir().join("xenolith-range.bin");
    std::fs::write(&path, vec![0x60u8; 64]).unwrap();

    let (ok, _, stderr) = run!(
        "disasm",
        "--raw",
        "--base",
        "0x82000000",
        "--start",
        "0x90000000",
        path.to_str().unwrap()
    );
    let _ = std::fs::remove_file(&path);

    assert!(!ok, "an unmapped range should fail");
    assert!(!stderr.contains("panicked"), "the tool panicked: {stderr}");
}

/// A word that decodes and a word that does not must both appear, so that a
/// sweep of a range never silently skips anything.
#[test]
fn disassembles_a_raw_image_marking_undecoded_words() {
    let path = std::env::temp_dir().join("xenolith-disasm.bin");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x3860_0005u32.to_be_bytes()); // a load immediate
    bytes.extend_from_slice(&0x17ff_ffffu32.to_be_bytes()); // an unassigned primary
    std::fs::write(&path, &bytes).unwrap();

    let (ok, stdout, _) = run!(
        "disasm",
        "--raw",
        "--base",
        "0x82000000",
        "--start",
        "0x82000000",
        "--length",
        "8",
        path.to_str().unwrap()
    );
    let _ = std::fs::remove_file(&path);

    assert!(ok, "disassembly should succeed");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 2, "one line per word: {stdout}");
    assert!(lines[0].contains("0x82000000"), "{stdout}");
    assert!(
        lines[1].contains(".long"),
        "the undecoded word was not marked"
    );
}

#[test]
fn a_sweep_reports_coverage() {
    let path = std::env::temp_dir().join("xenolith-sweep.bin");
    let mut bytes = Vec::new();
    for _ in 0..16 {
        bytes.extend_from_slice(&0x3860_0005u32.to_be_bytes());
    }
    bytes.extend_from_slice(&0x17ff_ffffu32.to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();

    let (ok, stdout, _) = run!("disasm", "--raw", "--sweep", path.to_str().unwrap());
    let _ = std::fs::remove_file(&path);

    assert!(ok, "sweep should succeed");
    assert!(stdout.contains("swept 17 instruction words"), "{stdout}");
    assert!(stdout.contains("decoded"), "{stdout}");
    assert!(stdout.contains("undecoded"), "{stdout}");
    assert!(
        stdout.contains("17ffffff"),
        "the undecoded encoding was not listed: {stdout}"
    );
}

#[test]
fn top_level_help_lists_analyze() {
    let (ok, stdout, _) = run!("--help");

    assert!(ok, "help should succeed");
    assert!(stdout.contains("analyze"), "help did not list analyze");
}

#[test]
fn analyze_help_documents_its_arguments() {
    let (ok, stdout, _) = run!("analyze", "--help");

    assert!(ok, "analyze help should succeed");
    for flag in ["--raw", "--base", "--key-file", "--functions", "--tables"] {
        assert!(stdout.contains(flag), "{flag} not documented: {stdout}");
    }
}

#[test]
fn analyze_names_a_file_it_cannot_read_as_a_container() {
    let path = std::env::temp_dir().join("xenolith-analyze-garbage.bin");
    std::fs::write(&path, b"this is definitely not a xex file").unwrap();

    let (ok, _, stderr) = run!("analyze", path.to_str().unwrap());

    assert!(!ok, "analyzing a non container should fail");
    assert!(
        stderr.contains("xenolith-analyze-garbage.bin"),
        "the error did not name the file: {stderr}"
    );
}

#[test]
fn analyze_refuses_an_image_with_no_executable_words() {
    let path = std::env::temp_dir().join("xenolith-analyze-empty.bin");
    std::fs::write(&path, b"").unwrap();

    let (ok, _, stderr) = run!("analyze", "--raw", path.to_str().unwrap());

    assert!(!ok, "an image with no code should fail");
    assert!(
        stderr.contains("no executable words"),
        "the error did not say what was wrong: {stderr}"
    );
}

/// A switch whose recovery can be worked out by hand, so the reported table can
/// be checked against what the code says rather than against itself.
///
/// ```text
/// 0x82000000  mflr   r12                  a prologue, so the function is found
/// 0x82000004  stwu   r1, -32(r1)
/// 0x82000008  cmpli  r10, 3               four entries
/// 0x8200000c  bc     0x82000038           the default
/// 0x82000010  lis    r12, 0x8200
/// 0x82000014  addi   r12, r12, 0x48       the table
/// 0x82000018  lbzx   r0, r12, r10         entry = table[r10]
/// 0x8200001c  rlwinm r0, r0, 2, 0, 29     entry * 4
/// 0x82000020  lis    r12, 0x8200
/// 0x82000024  addi   r12, r12, 0x28       the base
/// 0x82000028  add    r12, r12, r0         target = base + entry * 4
/// 0x8200002c  mtctr  r12
/// 0x82000030  bctr
/// ```
///
/// The table holds 3, 5, 6 and 7, so the targets are the base plus 12, 20, 24
/// and 28.
#[test]
fn analyze_reports_a_jump_table_worked_out_by_hand() {
    const WORDS: [u32; 19] = [
        0x7d88_02a6,
        0x9421_ffe0,
        0x280a_0003,
        0x4181_002c,
        0x3d80_8200,
        0x398c_0048,
        0x7c0c_50ae,
        0x5400_103a,
        0x3d80_8200,
        0x398c_0028,
        0x7d8c_0214,
        0x7d89_03a6,
        0x4e80_0420,
        0x4e80_0020,
        0x4e80_0020,
        0x4e80_0020,
        0x4e80_0020,
        0x4e80_0020,
        0x0305_0607,
    ];

    let path = std::env::temp_dir().join("xenolith-analyze-switch.bin");
    let bytes: Vec<u8> = WORDS.iter().flat_map(|word| word.to_be_bytes()).collect();
    std::fs::write(&path, &bytes).unwrap();

    let (ok, stdout, stderr) = run!(
        "analyze",
        "--raw",
        "--base",
        "0x82000000",
        "--tables",
        path.to_str().unwrap()
    );

    assert!(ok, "analyzing a crafted image should succeed: {stderr}");
    assert!(
        stdout.contains("tables recovered            1"),
        "the table was not recovered: {stdout}"
    );
    assert!(
        stdout.contains("resolved                  1"),
        "the branch was not reported resolved: {stdout}"
    );
    assert!(
        stdout.contains("unresolved                0"),
        "the branch was reported unresolved: {stdout}"
    );
    assert!(
        stdout.contains("branch 0x82000030  index r10  table 0x82000048  default 0x82000038"),
        "the table was not reported as expected: {stdout}"
    );
    for target in ["0x82000034", "0x8200003c", "0x82000040", "0x82000044"] {
        assert!(stdout.contains(target), "{target} missing from: {stdout}");
    }
}
